//! WKWebView is confined to this helper's main thread and a fresh nonpersistent store.
use super::*;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::*;
use objc2_foundation::*;
use objc2_web_kit::*;
use std::{cell::RefCell, rc::Rc, time::Instant};

type PendingResource = (
    Retained<ProtocolObject<dyn WKURLSchemeTask>>,
    mpsc::Receiver<Result<crate::html_network::Resource, String>>,
);
#[derive(Default)]
struct GuardState {
    resources: RefCell<Vec<PendingResource>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = GuardState]
    struct Guard;

    unsafe impl NSObjectProtocol for Guard {}

    unsafe impl WKURLSchemeHandler for Guard {
        #[unsafe(method(webView:startURLSchemeTask:))]
        unsafe fn resource(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static ACTIVE: AtomicUsize = AtomicUsize::new(0);
            let url = unsafe { task.request().URL() }
                .and_then(|u| u.absoluteString())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if ACTIVE.fetch_add(1, Ordering::AcqRel) >= 32 {
                ACTIVE.fetch_sub(1, Ordering::AcqRel);
                unsafe {
                    task.didFailWithError(&NSError::errorWithDomain_code_userInfo(
                        &NSString::from_str("ShinaHTML"),
                        1,
                        None,
                    ));
                }
                return;
            }
            let (send, receive) = mpsc::channel();
            let owned = unsafe { Retained::retain(task as *const _ as *mut _) }
                .expect("retained resource task");
            self.ivars().resources.borrow_mut().push((owned, receive));
            std::thread::spawn(move || {
                let result = crate::html_network::resource(&url);
                ACTIVE.fetch_sub(1, Ordering::AcqRel);
                let _ = send.send(result);
            });
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn stop_resource(
            &self,
            _view: &WKWebView,
            stopped: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            self.ivars()
                .resources
                .borrow_mut()
                .retain(|(task, _)| !std::ptr::eq(&**task, stopped));
        }
    }

    unsafe impl WKNavigationDelegate for Guard {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn navigation(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            reply: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let url = unsafe { action.request().URL() }
                .and_then(|u| u.absoluteString())
                .map(|u| u.to_string())
                .unwrap_or_default();
            // Only the two in-memory documents may navigate. Frames, redirects, form
            // submissions, downloads and script navigations cannot leave this page.
            let initial = matches!(url.split('#').next(), Some("about:blank" | "about:srcdoc"))
                && !unsafe { action.shouldPerformDownload() };
            reply.call((if initial {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }
    }

    unsafe impl WKUIDelegate for Guard {
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        unsafe fn new_window(
            &self,
            _view: &WKWebView,
            _config: &WKWebViewConfiguration,
            _action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> *mut WKWebView {
            std::ptr::null_mut()
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        unsafe fn media(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            reply: &block2::DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            reply.call((WKPermissionDecision::Deny,));
        }

        #[unsafe(method(webView:requestDeviceOrientationAndMotionPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        unsafe fn motion(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            reply: &block2::DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            reply.call((WKPermissionDecision::Deny,));
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        unsafe fn files(
            &self,
            _view: &WKWebView,
            _parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            reply: &block2::DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            reply.call((std::ptr::null_mut(),));
        }
    }

    unsafe impl WKScriptMessageHandler for Guard {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        unsafe fn link(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // Only the trusted outer frame can use the link bridge. The untrusted srcdoc
            // receives no handler, and the outer script requires transient user activation.
            if !unsafe { message.frameInfo().isMainFrame() } {
                return;
            }
            let body = unsafe { message.body() };
            if let Some(url) = body.downcast_ref::<NSString>() {
                let text = url.to_string();
                if url::Url::parse(&text)
                    .ok()
                    .is_some_and(|u| matches!(u.scheme(), "http" | "https"))
                    && let Some(url) = NSURL::URLWithString(url)
                {
                    NSWorkspace::sharedWorkspace().openURL(&url);
                }
            }
        }
    }
);

impl Guard {
    fn drain_resources(&self) {
        let mut ready = vec![];
        self.ivars()
            .resources
            .borrow_mut()
            .retain(|(task, receive)| match receive.try_recv() {
                Ok(result) => {
                    ready.push((task.clone(), result));
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    ready.push((task.clone(), Err("Resource loader closed.".into())));
                    false
                }
                Err(_) => true,
            });
        for (task, result) in ready {
            unsafe {
                if let Ok(resource) = result
                    && let Some(url) = task.request().URL()
                {
                    let headers = NSDictionary::from_slices(
                        &[
                            &*NSString::from_str("Content-Type"),
                            &*NSString::from_str("Access-Control-Allow-Origin"),
                            &*NSString::from_str("Cache-Control"),
                        ],
                        &[
                            &*NSString::from_str(&resource.mime),
                            &*NSString::from_str("*"),
                            &*NSString::from_str("no-store"),
                        ],
                    );
                    if let Some(response) =
                        NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
                            NSHTTPURLResponse::alloc(),
                            &url,
                            200,
                            None,
                            Some(&headers),
                        )
                    {
                        task.didReceiveResponse(&response);
                        task.didReceiveData(&NSData::with_bytes(&resource.bytes));
                        task.didFinish();
                        continue;
                    }
                }
                task.didFailWithError(&NSError::errorWithDomain_code_userInfo(
                    &NSString::from_str("ShinaHTML"),
                    2,
                    None,
                ));
            }
        }
    }
}

/// Fail closed before loading any untrusted bytes. Standard HTTP, WebSocket and
/// file requests are blocked even when WebKit would bypass a configured proxy.
unsafe fn install_rules(
    controller: &WKUserContentController,
    mtm: MainThreadMarker,
) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("shina-html-rules-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let store = unsafe { WKContentRuleListStore::storeWithURL(Some(&url), mtm) }
        .ok_or("No content rule store.")?;
    let compiled = Rc::new(RefCell::new(None));
    let out = compiled.clone();
    let done = RcBlock::new(move |rule: *mut WKContentRuleList, error: *mut NSError| {
        *out.borrow_mut() = Some(unsafe { Retained::retain(rule) }.ok_or_else(|| {
            unsafe { error.as_ref() }
                .map(|error| error.localizedDescription().to_string())
                .unwrap_or_else(|| "Network isolation rule compilation failed.".into())
        }));
    });
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str("ShinaHTMLNetworkIsolation")),
            Some(&NSString::from_str(r#"[{"trigger":{"url-filter":"^http:"},"action":{"type":"block"}},{"trigger":{"url-filter":"^https:"},"action":{"type":"block"}},{"trigger":{"url-filter":"^ws:"},"action":{"type":"block"}},{"trigger":{"url-filter":"^wss:"},"action":{"type":"block"}},{"trigger":{"url-filter":"^file:"},"action":{"type":"block"}},{"trigger":{"url-filter":"^ftp:"},"action":{"type":"block"}}]"#)),Some(&done));
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while compiled.borrow().is_none() && Instant::now() < deadline {
        pump();
    }
    let rule = compiled
        .borrow_mut()
        .take()
        .ok_or("Network isolation rule compilation timed out.")??;
    unsafe {
        controller.addContentRuleList(&rule);
    }
    // Only this helper's trusted compiled rule cache is removed.
    let _ = std::fs::remove_dir_all(&path);
    Ok(())
}

fn pump() {
    NSRunLoop::currentRunLoop().runMode_beforeDate(
        unsafe { NSDefaultRunLoopMode },
        &NSDate::dateWithTimeIntervalSinceNow(0.01),
    );
}

// Compatibility only. The mandatory content rule and Rust loader enforce security
// even if a page removes these setters or constructs a URL another way.
const RESOURCE_SETTERS: &str = include_str!("../assets/html/resources.js");

fn eval(view: &WKWebView, script: &str, timeout: Duration) -> Option<Value> {
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    let block = RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
        let value = unsafe { value.as_ref() }
            .and_then(|v| v.downcast_ref::<NSString>())
            .and_then(|s| serde_json::from_str::<Value>(&s.to_string()).ok());
        *out.borrow_mut() = Some(value);
    });
    unsafe {
        view.evaluateJavaScript_completionHandler(&NSString::from_str(script), Some(&block));
    }
    let deadline = Instant::now() + timeout;
    while result.borrow().is_none() && Instant::now() < deadline {
        pump();
    }
    result.borrow_mut().take().flatten()
}

pub fn run() -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("WKWebView must run on the main thread.")?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let input: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    let pdf = pdf_bytes(&input)?;
    let html = input["html"].as_str().ok_or("Missing HTML.")?;
    let maximum = if input["attachmentPreview"] == true {
        crate::attachments::MAX_FILE_BYTES.div_ceil(3) as usize * 4 + 4096
    } else {
        crate::html_render::MAX_PAGE
    };
    if html.len() > maximum {
        return Err("HTML exceeds 25 MiB.".into());
    }
    let width = input["width"].as_u64().unwrap_or(728).clamp(240, 1600) as f64;
    let capture = input["capture"] == true;
    let dark = input["dark"] == true;
    let application = NSApplication::sharedApplication(mtm);
    application.setActivationPolicy(if capture {
        NSApplicationActivationPolicy::Prohibited
    } else {
        NSApplicationActivationPolicy::Regular
    });
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        let store = WKWebsiteDataStore::nonPersistentDataStore(mtm);
        config.setWebsiteDataStore(&store);
        let prefs = config.preferences();
        prefs.setJavaScriptCanOpenWindowsAutomatically(false);
        // WebRTC can bypass TCP proxies with UDP. WebKit exposes this setting through KVC.
        prefs.setValue_forKey(
            Some(&NSNumber::new_bool(false)),
            &NSString::from_str("peerConnectionEnabled"),
        );
        prefs.setValue_forKey(
            Some(&NSNumber::new_bool(false)),
            &NSString::from_str("javaScriptCanAccessClipboard"),
        );
        prefs.setValue_forKey(
            Some(&NSNumber::new_bool(false)),
            &NSString::from_str("allowFileAccessFromFileURLs"),
        );
        config.setValue_forKey(
            Some(&NSNumber::new_bool(false)),
            &NSString::from_str("allowUniversalAccessFromFileURLs"),
        );
        config.setAllowsAirPlayForMediaPlayback(false);
        let allocated = Guard::alloc(mtm).set_ivars(GuardState::default());
        let guard: Retained<Guard> = msg_send![super(allocated), init];
        let controller = config.userContentController();
        install_rules(&controller, mtm)?;
        controller.addUserScript(
            &WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(RESOURCE_SETTERS),
                WKUserScriptInjectionTime::AtDocumentStart,
                false,
            ),
        );
        for scheme in ["shina-http", "shina-https"] {
            config.setURLSchemeHandler_forURLScheme(
                Some(ProtocolObject::from_ref(&*guard)),
                &NSString::from_str(scheme),
            );
        }
        controller.addScriptMessageHandler_name(
            ProtocolObject::from_ref(&*guard),
            &NSString::from_str("link"),
        );
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 800.0));
        let view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config);
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*guard)));
        view.setUIDelegate(Some(ProtocolObject::from_ref(&*guard)));
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(
                NSPoint::new(if capture { -20000.0 } else { 100.0 }, 100.0),
                frame.size,
            ),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::Miniaturizable,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.setTitle(&NSString::from_str(
            input["title"].as_str().unwrap_or("Shina HTML"),
        ));
        window.setContentView(Some(&view));
        view.unregisterDraggedTypes();
        if capture {
            window.orderBack(None);
        } else {
            window.makeKeyAndOrderFront(None);
            application.activate();
        }
        if let Some(bytes) = &pdf {
            view.loadData_MIMEType_characterEncodingName_baseURL(
                &NSData::with_bytes(bytes),
                &NSString::from_str("application/pdf"),
                &NSString::from_str("UTF-8"),
                &NSURL::URLWithString(&NSString::from_str("about:blank")).unwrap(),
            );
        } else {
            let source = document(html, dark);
            view.loadHTMLString_baseURL(&NSString::from_str(&source), None);
        }
        if capture {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut measured = json!({"height":80,"consoleMessages":[]});
            let mut previous = 0;
            let mut stable = Instant::now();
            while Instant::now() < deadline {
                guard.drain_resources();
                pump();
                if let Some(value) = eval(
                    &view,
                    "JSON.stringify(window.shinaState)",
                    Duration::from_millis(100),
                ) {
                    measured = value;
                    let height = measured["height"].as_u64().unwrap_or(80);
                    if height != previous {
                        previous = height;
                        stable = Instant::now();
                    }
                    if !view.isLoading()
                        && measured["ready"] == true
                        && stable.elapsed() > Duration::from_millis(250)
                        && previous > 0
                    {
                        break;
                    }
                }
            }
            let height = measured["height"].as_u64().unwrap_or(80).clamp(1, 4000) as f64;
            view.setFrameSize(NSSize::new(width, height));
            // One additional paint turn after resizing. A busy page still has a deadline.
            let _ = eval(
                &view,
                "JSON.stringify(window.shinaState)",
                Duration::from_millis(100),
            );
            let snapshot = WKSnapshotConfiguration::new(mtm);
            snapshot.setRect(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width, height),
            ));
            snapshot.setSnapshotWidth(Some(&NSNumber::new_f64(width * 2.0)));
            snapshot.setAfterScreenUpdates(false);
            let png: Rc<RefCell<Option<Result<Vec<u8>, String>>>> = Default::default();
            let out = png.clone();
            let done = RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
                let encoded = image
                    .as_ref()
                    .and_then(|image| image.TIFFRepresentation())
                    .and_then(|data| {
                        NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data)
                    })
                    .and_then(|rep| {
                        rep.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    })
                    .map(|data| data.to_vec())
                    .ok_or("WKWebView snapshot failed.".into());
                *out.borrow_mut() = Some(encoded);
            });
            view.takeSnapshotWithConfiguration_completionHandler(Some(&snapshot), &done);
            let deadline = Instant::now() + Duration::from_secs(2);
            while png.borrow().is_none() && Instant::now() < deadline {
                guard.drain_resources();
                pump();
            }
            let png = png
                .borrow_mut()
                .take()
                .ok_or("WK snapshot deadline exceeded.")??;
            println!(
                "{}",
                json!({"png":crate::attachments::encode_base64(&png),
                "contentHeight":measured["height"],"consoleMessages":measured["consoleMessages"]})
            );
            return Ok(());
        }
        println!("{}", json!({"ready":true}));
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        loop {
            guard.drain_resources();
            if !window.isVisible() && !window.isMiniaturized() {
                break;
            }
            // Native events wake this wait immediately. Pipe/theme changes have a
            // 250 ms bound without polling an idle live window at capture frequency.
            if let Some(event) = application.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::dateWithTimeIntervalSinceNow(0.25)),
                NSDefaultRunLoopMode,
                true,
            ) {
                application.sendEvent(&event);
            }
            // Dispatch Cocoa events while retaining the native window after its thread tab closes.
            while let Some(event) = application.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::distantPast()),
                NSDefaultRunLoopMode,
                true,
            ) {
                application.sendEvent(&event);
            }
            match receive.try_recv() {
                Ok(line) => {
                    if let Ok(command) = serde_json::from_str::<Value>(&line) {
                        if command["focus"] == true {
                            if window.isMiniaturized() {
                                window.deminiaturize(None);
                            }
                            window.makeKeyAndOrderFront(None);
                            application.activate();
                        }
                        if let Some(dark) = command["dark"].as_bool() {
                            let theme = crate::ui::html_theme(dark);
                            let _ = eval(
                                &view,
                                &format!("window.shinaTheme({theme});JSON.stringify(true)"),
                                Duration::from_millis(100),
                            );
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(_) => {}
            }
        }
    }
    Ok(())
}
