//! WebView2 in an owned helper with an InPrivate controller and an opaque custom scheme.
use super::*;
use std::{rc::Rc, time::Instant};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    Win32::{
        Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::{
            Com::{StructuredStorage::CreateStreamOnHGlobal, *},
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Shell::{SHCreateMemStream, ShellExecuteW},
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, Interface, PCWSTR, PWSTR, w},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
static CLOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLOSE {
        CLOSED.store(true, std::sync::atomic::Ordering::Release);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn pump() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    std::thread::sleep(Duration::from_millis(5));
}

fn wait<T>(receiver: &mpsc::Receiver<T>, duration: Duration) -> Result<T> {
    let deadline = Instant::now() + duration;
    loop {
        if let Ok(value) = receiver.try_recv() {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err("WebView2 operation timed out.".into());
        }
        pump();
    }
}

fn script(view: &ICoreWebView2, source: &str) -> Result<Value> {
    let (send, receive) = mpsc::channel();
    unsafe {
        view.ExecuteScript(
            &HSTRING::from(source),
            &ExecuteScriptCompletedHandler::create(Box::new(move |status, result| {
                let value = status.map(|_| result);
                let _ = send.send(value);
                Ok(())
            })),
        )?;
    }
    let result = wait(&receive, Duration::from_millis(200))??;
    Ok(serde_json::from_str(&result)?)
}

fn cdp(view: &ICoreWebView2, method: &str, params: &str) -> Result<()> {
    let (send, receive) = mpsc::channel();
    unsafe {
        view.CallDevToolsProtocolMethod(
            &HSTRING::from(method),
            &HSTRING::from(params),
            &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, _| {
                let _ = send.send(status);
                Ok(())
            })),
        )?;
    }
    wait(&receive, Duration::from_secs(1))??;
    Ok(())
}

const AUTO_ATTACH: &str = r#"{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,"filter":[{"type":"iframe","exclude":false},{"exclude":true}]}"#;

/// Cross-process iframes pause before script execution. Install file-chooser denial
/// and recursive target attachment before resuming each one. Errors leave it paused.
fn secure_frame(view: ICoreWebView2_11, session: String, step: usize) -> windows::core::Result<()> {
    let methods = [
        ("Page.setInterceptFileChooserDialog", r#"{"enabled":true}"#),
        ("Target.setAutoAttach", AUTO_ATTACH),
        ("Runtime.runIfWaitingForDebugger", "{}"),
    ];
    let Some((method, params)) = methods.get(step) else {
        return Ok(());
    };
    let next_view = view.clone();
    let next_session = session.clone();
    unsafe {
        view.CallDevToolsProtocolMethodForSession(
            &HSTRING::from(session),
            &HSTRING::from(*method),
            &HSTRING::from(*params),
            &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, _| {
                status?;
                secure_frame(next_view.clone(), next_session.clone(), step + 1)
            })),
        )
    }
}

fn uri(get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> windows::core::Result<String> {
    let mut raw = PWSTR::null();
    get(&mut raw)?;
    Ok(CoTaskMemPWSTR::from(raw).to_string())
}

pub fn run() -> std::result::Result<(), String> {
    native().map_err(|e| e.to_string())
}

fn native() -> Result<()> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let input: Value = serde_json::from_str(&line)?;
    let pdf = pdf_bytes(&input)
        .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80070057u32 as i32), e))?;
    let html = input["html"].as_str().ok_or("Missing HTML.")?;
    let maximum = if input["attachmentPreview"] == true {
        crate::attachments::MAX_FILE_BYTES.div_ceil(3) as usize * 4 + 4096
    } else {
        crate::html_render::MAX_PAGE
    };
    if html.len() > maximum {
        return Err("HTML exceeds 25 MiB.".into());
    }
    let width = input["width"].as_u64().unwrap_or(728).clamp(240, 1600) as i32;
    let capture = input["capture"] == true;
    let dark = input["dark"] == true;
    let profile = std::env::temp_dir().join(format!("shina-html-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&profile)?;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("ShinaHtmlWindow");
        let registered = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        });
        if registered == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            &HSTRING::from(input["title"].as_str().unwrap_or("Shina HTML")),
            WS_OVERLAPPEDWINDOW,
            if capture { -20000 } else { 100 },
            100,
            width,
            800,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let options = CoreWebView2EnvironmentOptions::default();
        let port = crate::html_network::proxy()?;
        options.set_additional_browser_arguments(format!(
            "--proxy-server=http://127.0.0.1:{port} --proxy-bypass-list=<-loopback> --force-webrtc-ip-handling-policy=disable_non_proxied_udp --auth-schemes=basic,digest --disable-quic --disable-features=FileSystemAccessAPI,FileSystemAccessLocal,WebBluetooth,WebUSB,WebSerial --disable-file-system"));
        options.set_allow_single_sign_on_using_os_primary_account(false);
        options.set_are_browser_extensions_enabled(false);
        let scheme = CoreWebView2CustomSchemeRegistration::new("shina-page".into());
        scheme.set_has_authority_component(false);
        scheme.set_treat_as_secure(false);
        options.set_scheme_registrations(vec![Some(scheme.into())]);
        let options: ICoreWebView2EnvironmentOptions = options.into();
        let (send, receive) = mpsc::channel();
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &HSTRING::from(profile.to_string_lossy().as_ref()),
            &options,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |status, environment| {
                    let _ = send.send(status.and_then(|_| {
                        environment.ok_or_else(|| windows::core::Error::from_win32())
                    }));
                    Ok(())
                },
            )),
        )?;
        let environment = wait(&receive, Duration::from_secs(2))??;
        let environment10: ICoreWebView2Environment10 = environment.cast()?;
        let controller_options = environment10.CreateCoreWebView2ControllerOptions()?;
        controller_options.SetIsInPrivateModeEnabled(true)?;
        let (send, receive) = mpsc::channel();
        environment10.CreateCoreWebView2ControllerWithOptions(
            window,
            &controller_options,
            &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |status, controller| {
                    let _ = send.send(status.and_then(|_| {
                        controller.ok_or_else(|| windows::core::Error::from_win32())
                    }));
                    Ok(())
                },
            )),
        )?;
        let controller = wait(&receive, Duration::from_secs(2))??;
        let controller4: ICoreWebView2Controller4 = controller.cast()?;
        controller4.SetAllowExternalDrop(false)?;
        let controller3: ICoreWebView2Controller3 = controller.cast()?;
        controller3.SetShouldDetectMonitorScaleChanges(false)?;
        controller3.SetRasterizationScale(if capture { 2.0 } else { 1.0 })?;
        controller3.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)?;
        controller.SetBounds(RECT {
            left: 0,
            top: 0,
            right: width * if capture { 2 } else { 1 },
            bottom: 800 * if capture { 2 } else { 1 },
        })?;
        controller.SetIsVisible(true)?;
        let view = controller.CoreWebView2()?;
        let settings = view.Settings()?;
        settings.SetAreHostObjectsAllowed(false)?;
        settings.SetAreDefaultScriptDialogsEnabled(false)?;
        settings.SetAreDefaultContextMenusEnabled(false)?;
        settings.SetAreDevToolsEnabled(false)?;
        settings.SetIsStatusBarEnabled(false)?;
        settings.SetIsZoomControlEnabled(false)?;
        let mut token = 0;
        let view10: ICoreWebView2_10 = view.cast()?;
        view10.add_BasicAuthenticationRequested(
            &BasicAuthenticationRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let view5: ICoreWebView2_5 = view.cast()?;
        view5.add_ClientCertificateRequested(
            &ClientCertificateRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        view.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        view.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let view4: ICoreWebView2_4 = view.cast()?;
        view4.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let navigation = NavigationStartingEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                let url = uri(|raw| args.Uri(raw))?;
                if !matches!(
                    url.split('#').next(),
                    Some("shina-page:page" | "about:srcdoc" | "about:blank")
                ) {
                    args.SetCancel(true)?;
                }
            }
            Ok(())
        }));
        view.add_NavigationStarting(&navigation, &mut token)?;
        view.add_FrameNavigationStarting(&navigation, &mut token)?;
        // File chooser interception is mandatory, not a best-effort script override.
        cdp(
            &view,
            "Page.setInterceptFileChooserDialog",
            r#"{"enabled":true}"#,
        )?;
        let view11: ICoreWebView2_11 = view.cast()?;
        let targets = view.GetDevToolsProtocolEventReceiver(w!("Target.attachedToTarget"))?;
        targets.add_DevToolsProtocolEventReceived(
            &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let raw = uri(|raw| args.ParameterObjectAsJson(raw))?;
                    if let Ok(value) = serde_json::from_str::<Value>(&raw)
                        && let Some(session) = value["sessionId"].as_str()
                    {
                        secure_frame(view11.clone(), session.to_owned(), 0)?;
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        cdp(&view, "Target.setAutoAttach", AUTO_ATTACH)?;
        let body = Rc::new(
            if let Some(bytes) = pdf {
                pdf_document(&bytes)
            } else {
                document(html, dark)
            }
            .into_bytes(),
        );
        let resource_environment = environment.clone();
        view.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        view.add_WebResourceRequested(&WebResourceRequestedEventHandler::create(Box::new(move |_,args| {
            if let Some(args) = args {
                let request = args.Request()?;
                let url = uri(|raw|request.Uri(raw))?;
                if url == "shina-page:page" {
                    let stream = SHCreateMemStream(Some(&body)).ok_or_else(||windows::core::Error::from_win32())?;
                    let response = resource_environment.CreateWebResourceResponse(&stream,200,w!("OK"),
                        w!("Content-Type: text/html; charset=utf-8\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store"))?;
                    args.SetResponse(&response)?;
                } else if !crate::html_render::public_url(&url) && !url.starts_with("data:") && !url.starts_with("blob:") {
                    let response = resource_environment.CreateWebResourceResponse(None,403,w!("Forbidden"),w!("Content-Length: 0"))?;
                    args.SetResponse(&response)?;
                }
                // Public requests use the CONNECT proxy, which checks DNS and pins the IP.
            }
            Ok(())
        })),&mut token)?;
        view.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let source = uri(|raw| args.Source(raw))?;
                    if source != "shina-page:page" {
                        return Ok(());
                    }
                    let raw = uri(|raw| args.WebMessageAsJson(raw))?;
                    if let Ok(value) = serde_json::from_str::<Value>(&raw)
                        && let Some(link) = value["link"].as_str()
                        && url::Url::parse(link)
                            .ok()
                            .is_some_and(|u| matches!(u.scheme(), "http" | "https"))
                    {
                        let _ = ShellExecuteW(
                            None,
                            w!("open"),
                            &HSTRING::from(link),
                            None,
                            None,
                            SW_SHOWNORMAL,
                        );
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        let _ = ShowWindow(window, if capture { SW_SHOWNOACTIVATE } else { SW_SHOW });
        view.Navigate(w!("shina-page:page"))?;
        if capture {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut measured = json!({"height":80,"consoleMessages":[]});
            let mut previous = 0;
            let mut stable = Instant::now();
            while Instant::now() < deadline {
                pump();
                if let Ok(value) = script(&view, "window.shinaState") {
                    if value["height"].as_u64().is_some() {
                        measured = value;
                        let height = measured["height"].as_u64().unwrap_or(80);
                        if height != previous {
                            previous = height;
                            stable = Instant::now();
                        }
                        if measured["ready"] == true
                            && stable.elapsed() > Duration::from_millis(250)
                            && previous > 0
                        {
                            break;
                        }
                    }
                }
            }
            let height = measured["height"].as_u64().unwrap_or(80).clamp(1, 4000) as i32;
            controller.SetBounds(RECT {
                left: 0,
                top: 0,
                right: width * 2,
                bottom: height * 2,
            })?;
            let _ = script(&view, "window.shinaState");
            let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
            let (send, receive) = mpsc::channel();
            view.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &CapturePreviewCompletedHandler::create(Box::new(move |status| {
                    let _ = send.send(status);
                    Ok(())
                })),
            )?;
            wait(&receive, Duration::from_secs(2))??;
            let mut stat = STATSTG::default();
            stream.Stat(&mut stat, STATFLAG_NONAME)?;
            if stat.cbSize > crate::html_render::MAX_PAGE as u64 {
                return Err("Snapshot exceeds 25 MiB.".into());
            }
            stream.Seek(0, STREAM_SEEK_SET, None)?;
            let mut png = vec![0; stat.cbSize as usize];
            let mut count = 0;
            stream
                .Read(png.as_mut_ptr().cast(), png.len() as u32, Some(&mut count))
                .ok()?;
            png.truncate(count as usize);
            println!(
                "{}",
                json!({"png":crate::attachments::encode_base64(&png),"contentHeight":measured["height"],"consoleMessages":measured["consoleMessages"]})
            );
        } else {
            println!("{}", json!({"ready":true}));
            std::io::stdout().flush()?;
            let (send, receive) = mpsc::channel();
            let native_window = window.0 as usize;
            std::thread::spawn(move || {
                for line in std::io::stdin().lock().lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    if send.send(line).is_err() {
                        break;
                    }
                    let _ = PostMessageW(
                        Some(HWND(native_window as *mut _)),
                        WM_APP,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
                let _ = PostMessageW(
                    Some(HWND(native_window as *mut _)),
                    WM_APP,
                    WPARAM(0),
                    LPARAM(0),
                );
            });
            loop {
                pump();
                if CLOSED.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let mut bounds = RECT::default();
                GetClientRect(window, &mut bounds)?;
                controller.SetBounds(bounds)?;
                match receive.try_recv() {
                    Ok(line) => {
                        if let Ok(command) = serde_json::from_str::<Value>(&line) {
                            if command["focus"] == true {
                                let _ = ShowWindow(window, SW_RESTORE);
                                let _ = SetForegroundWindow(window);
                            }
                            if let Some(dark) = command["dark"].as_bool() {
                                let _ = script(
                                    &view,
                                    &format!("window.shinaTheme({})", crate::ui::html_theme(dark)),
                                );
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(_) => {}
                }
                // Native browser/window messages and pipe commands wake this immediately.
                let _ = MsgWaitForMultipleObjectsEx(None, 1000, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }
        controller.Close()?;
        DestroyWindow(window)?;
        CoUninitialize();
    }
    let _ = std::fs::remove_dir_all(&profile);
    Ok(())
}
