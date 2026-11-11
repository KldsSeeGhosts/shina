//! Windows notification-area owner. A dedicated message loop receives tray and power events.
#[cfg(windows)]
mod windows {
    use rune::Waker;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    };

    const CALLBACK: u32 = WM_APP + 1;
    const REFRESH: u32 = WM_APP + 2;
    const OPEN: usize = 1;
    const QUIT: usize = 2;

    struct Context {
        count: Arc<AtomicUsize>,
        resumed: Arc<AtomicBool>,
        action: Arc<AtomicUsize>,
        wake: Waker,
        icon: HICON,
        added: bool,
        taskbar_created: u32,
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    unsafe fn notify(hwnd: HWND, context: &mut Context) -> bool {
        let encoded = context.count.load(Ordering::Relaxed);
        if encoded == 0 {
            if context.added {
                let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
                data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = hwnd;
                data.uID = 1;
                unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                context.added = false;
            }
            return true;
        }
        let count = encoded - 1;
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = CALLBACK;
        data.hIcon = context.icon;
        let tip = wide(&format!("Shina: {count} working"));
        for (to, from) in data.szTip.iter_mut().zip(tip) {
            *to = from;
        }
        let ok =
            unsafe { Shell_NotifyIconW(if context.added { NIM_MODIFY } else { NIM_ADD }, &data) }
                != 0;
        context.added = ok;
        ok
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Context;
        if !ptr.is_null() {
            let context = unsafe { &mut *ptr };
            if message == context.taskbar_created {
                context.added = false;
                unsafe { notify(hwnd, context) };
                return 0;
            }
            match message {
                REFRESH => {
                    return isize::from(unsafe { notify(hwnd, context) });
                }
                WM_POWERBROADCAST if matches!(wparam, 7 | 18) => {
                    context.resumed.store(true, Ordering::Release);
                    (context.wake)();
                    return 1;
                }
                WM_POWERBROADCAST => {
                    (context.wake)();
                    return 1;
                }
                CALLBACK => {
                    if lparam as u32 == WM_LBUTTONDBLCLK {
                        context.action.store(OPEN, Ordering::Release);
                        (context.wake)();
                    } else if matches!(lparam as u32, WM_RBUTTONUP | WM_CONTEXTMENU) {
                        let menu = unsafe { CreatePopupMenu() };
                        if !menu.is_null() {
                            let count = context.count.load(Ordering::Relaxed).saturating_sub(1);
                            // TrackPopupMenu runs a nested message loop. Do not retain a
                            // mutable context borrow across that reentrant call.
                            let action = context.action.clone();
                            let wake = context.wake.clone();
                            unsafe {
                                AppendMenuW(menu, MF_STRING, OPEN, wide("Open Shina").as_ptr());
                                AppendMenuW(
                                    menu,
                                    MF_STRING | MF_DISABLED | MF_GRAYED,
                                    0,
                                    wide(&format!("{count} working")).as_ptr(),
                                );
                                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                                AppendMenuW(menu, MF_STRING, QUIT, wide("Quit").as_ptr());
                                let mut point = POINT { x: 0, y: 0 };
                                GetCursorPos(&mut point);
                                SetForegroundWindow(hwnd);
                                let selection = TrackPopupMenu(
                                    menu,
                                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                    point.x,
                                    point.y,
                                    0,
                                    hwnd,
                                    std::ptr::null(),
                                ) as usize;
                                DestroyMenu(menu);
                                PostMessageW(hwnd, WM_NULL, 0, 0);
                                if matches!(selection, OPEN | QUIT) {
                                    action.store(selection, Ordering::Release);
                                    wake();
                                }
                            }
                        }
                    }
                    return 0;
                }
                WM_CLOSE => {
                    context.count.store(0, Ordering::Relaxed);
                    unsafe {
                        notify(hwnd, context);
                        DestroyWindow(hwnd);
                    }
                    return 0;
                }
                WM_DESTROY => {
                    unsafe { PostQuitMessage(0) };
                    return 0;
                }
                _ => {}
            }
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    pub struct Tray {
        hwnd: usize,
        count: Arc<AtomicUsize>,
        resumed: Arc<AtomicBool>,
        action: Arc<AtomicUsize>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl Tray {
        pub fn start(wake: Waker) -> Result<Self, String> {
            let count = Arc::new(AtomicUsize::new(0));
            let resumed = Arc::new(AtomicBool::new(false));
            let action = Arc::new(AtomicUsize::new(0));
            let (ready, receive) = mpsc::sync_channel(1);
            let worker_count = count.clone();
            let worker_resumed = resumed.clone();
            let worker_action = action.clone();
            let worker = std::thread::spawn(move || unsafe {
                let mut context = Box::new(Context {
                    count: worker_count,
                    resumed: worker_resumed,
                    action: worker_action,
                    wake,
                    icon: std::ptr::null_mut(),
                    added: false,
                    taskbar_created: 0,
                });
                let instance = GetModuleHandleW(std::ptr::null());
                let class = wide("ShinaBackgroundOwner");
                let descriptor = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    hInstance: instance,
                    lpszClassName: class.as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassW(&descriptor);
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    let _ = ready.send(Err(std::io::Error::last_os_error().to_string()));
                    return;
                }
                // CreateIcon consumes bottom-up BGRA color-mask bytes.
                let source = crate::brand::pixels(false);
                let mut pixels: Vec<_> = source
                    .as_chunks::<{ crate::brand::SIZE * 4 }>()
                    .0
                    .iter()
                    .rev()
                    .flatten()
                    .copied()
                    .collect();
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                let mask = vec![0u8; crate::brand::SIZE.div_ceil(16) * 2 * crate::brand::SIZE];
                context.icon = CreateIcon(
                    instance,
                    crate::brand::SIZE as i32,
                    crate::brand::SIZE as i32,
                    1,
                    32,
                    mask.as_ptr(),
                    pixels.as_ptr(),
                );
                if context.icon.is_null() {
                    DestroyWindow(hwnd);
                    let _ = ready.send(Err("Could not create the notification-area icon.".into()));
                    return;
                }
                context.taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
                SetWindowLongPtrW(
                    hwnd,
                    GWLP_USERDATA,
                    (&mut *context as *mut Context) as isize,
                );
                let _ = ready.send(Ok(hwnd as usize));
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                DestroyIcon(context.icon);
            });
            let hwnd = receive
                .recv_timeout(std::time::Duration::from_secs(3))
                .map_err(|e| e.to_string())??;
            Ok(Self {
                hwnd,
                count,
                resumed,
                action,
                worker: Some(worker),
            })
        }

        pub fn update(&self, count: usize, visible: bool) {
            let encoded = if visible { count.saturating_add(1) } else { 0 };
            if self.count.swap(encoded, Ordering::AcqRel) != encoded {
                unsafe {
                    PostMessageW(self.hwnd as HWND, REFRESH, 0, 0);
                }
            }
        }
        /// Refuse to hide the app unless Explorer accepted its icon.
        pub fn show_for_close(&self, count: usize) -> bool {
            self.count.store(count.saturating_add(1), Ordering::Release);
            let mut result = 0;
            unsafe {
                SendMessageTimeoutW(
                    self.hwnd as HWND,
                    REFRESH,
                    0,
                    0,
                    SMTO_ABORTIFHUNG,
                    500,
                    &mut result,
                ) != 0
                    && result != 0
            }
        }
        pub fn resumed(&self) -> bool {
            self.resumed.swap(false, Ordering::AcqRel)
        }
        pub fn action(&self) -> usize {
            self.action.swap(0, Ordering::AcqRel)
        }
    }
    impl Drop for Tray {
        fn drop(&mut self) {
            unsafe {
                PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
            }
            if let Some(worker) = self.worker.take() {
                crate::process::join_for(worker, std::time::Duration::from_secs(1));
            }
        }
    }
}

#[cfg(windows)]
pub use windows::Tray;

#[cfg(not(windows))]
pub struct Tray;
#[cfg(not(windows))]
impl Tray {
    pub fn start(_: rune::Waker) -> Result<Self, String> {
        Err("Notification-area background mode is supported on Windows only.".into())
    }
    pub fn update(&self, _: usize, _: bool) {}
    pub fn resumed(&self) -> bool {
        false
    }
    pub fn action(&self) -> usize {
        0
    }
    pub fn show_for_close(&self, _: usize) -> bool {
        false
    }
}
