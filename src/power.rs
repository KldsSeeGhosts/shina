//! Power policies with testable transitions; native calls stay on their acquiring thread.
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAwake {
    #[default]
    Off,
    WhileRunning,
    OnAcPower,
}

impl KeepAwake {
    pub fn qualifies(self, working: bool, ac: Option<bool>) -> bool {
        working
            && match self {
                Self::Off => false,
                Self::WhileRunning => true,
                Self::OnAcPower => ac == Some(true),
            }
    }
}

pub trait Inhibitor {
    fn set(&mut self, active: bool) -> Result<(), String>;
    fn alive(&mut self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct Awake<I: Inhibitor = NativeInhibitor> {
    os: I,
    pub active: bool,
    pub error: Option<String>,
}

impl<I: Inhibitor> Awake<I> {
    pub fn update(&mut self, policy: KeepAwake, working: bool, ac: Option<bool>) {
        if self.active && !self.os.alive() {
            self.active = false;
            self.error =
                Some("The keep-awake helper exited. Sleep inhibition is unavailable.".into());
            return;
        }
        let desired = policy.qualifies(working, ac);
        if desired != self.active {
            match self.os.set(desired) {
                Ok(()) => {
                    self.active = desired;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
}
impl<I: Inhibitor> Drop for Awake<I> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.os.set(false);
        }
    }
}

#[derive(Default)]
pub struct NativeInhibitor {
    #[cfg(not(windows))]
    child: Option<crate::process::Process>,
}

impl Inhibitor for NativeInhibitor {
    fn alive(&mut self) -> bool {
        #[cfg(not(windows))]
        {
            self.child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
        }
        #[cfg(windows)]
        {
            true
        }
    }
    fn set(&mut self, active: bool) -> Result<(), String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{
                ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
            };
            let flags = ES_CONTINUOUS | if active { ES_SYSTEM_REQUIRED } else { 0 };
            if unsafe { SetThreadExecutionState(flags) } == 0 {
                return Err(format!(
                    "Keep-awake request failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        #[cfg(not(windows))]
        {
            if active {
                use std::process::{Command, Stdio};
                #[cfg(target_os = "macos")]
                let mut command = {
                    let mut cmd = Command::new("/usr/bin/caffeinate");
                    cmd.args(["-i", "-w", &std::process::id().to_string()]);
                    cmd
                };
                #[cfg(not(target_os = "macos"))]
                let mut command = {
                    let mut cmd = Command::new("systemd-inhibit");
                    cmd.args([
                        "--no-ask-password",
                        "--what=sleep",
                        "--mode=block",
                        "--why=Shina tasks are running",
                        "sleep",
                        "infinity",
                    ]);
                    cmd
                };
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                self.child = Some(
                    crate::process::Process::spawn(&mut command)
                        .map_err(|e| format!("Keep-awake is unavailable: {e}"))?,
                );
            } else if let Some(mut child) = self.child.take() {
                child.terminate();
                let _ = child.reap(Duration::from_secs(1));
            }
        }
        Ok(())
    }
}

pub fn ac_power() -> Option<bool> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut status: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut status) != 0 {
            return match status.ACLineStatus {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            };
        }
    }
    #[cfg(target_os = "macos")]
    unsafe {
        use std::ffi::{c_char, c_void};
        #[link(name = "IOKit", kind = "framework")]
        unsafe extern "C" {
            fn IOPSCopyPowerSourcesInfo() -> *const c_void;
            fn IOPSGetProvidingPowerSourceType(info: *const c_void) -> *const c_void;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFStringGetCString(
                string: *const c_void,
                buffer: *mut c_char,
                size: isize,
                encoding: u32,
            ) -> bool;
            fn CFRelease(value: *const c_void);
        }
        let info = IOPSCopyPowerSourcesInfo();
        if !info.is_null() {
            let kind = IOPSGetProvidingPowerSourceType(info);
            let mut buffer = [0u8; 64];
            let read = !kind.is_null()
                && CFStringGetCString(kind, buffer.as_mut_ptr().cast(), 64, 0x08000100);
            CFRelease(info);
            if read {
                return match std::ffi::CStr::from_ptr(buffer.as_ptr().cast()).to_bytes() {
                    b"AC Power" => Some(true),
                    b"Battery Power" => Some(false),
                    _ => None,
                };
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Ok(supplies) = std::fs::read_dir("/sys/class/power_supply") {
        let mut found = false;
        for supply in supplies.flatten() {
            if std::fs::read_to_string(supply.path().join("type"))
                .is_ok_and(|kind| kind.trim() == "Mains")
                && let Ok(online) = std::fs::read_to_string(supply.path().join("online"))
            {
                found = true;
                if online.trim() == "1" {
                    return Some(true);
                }
            }
        }
        if found {
            return Some(false);
        }
    }
    None
}

pub const AWAY_AFTER: Duration = Duration::from_secs(2 * 60 * 60);
pub fn away(idle: Option<Duration>) -> bool {
    idle.is_some_and(|idle| idle >= AWAY_AFTER)
}

pub fn idle_time() -> Option<Duration> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            System::SystemInformation::GetTickCount,
            UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
        };
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut info) != 0 {
            // GetLastInputInfo and GetTickCount share the wrapping 32-bit uptime clock.
            return Some(Duration::from_millis(u64::from(
                GetTickCount().wrapping_sub(info.dwTime),
            )));
        }
    }
    #[cfg(target_os = "macos")]
    unsafe {
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn CGEventSourceSecondsSinceLastEventType(state: i32, kind: u32) -> f64;
        }
        let seconds = CGEventSourceSecondsSinceLastEventType(0, u32::MAX);
        if seconds.is_finite() && seconds >= 0.0 {
            return Duration::try_from_secs_f64(seconds).ok();
        }
    }
    None
}

pub struct SleepClock {
    mono: Instant,
    wall: SystemTime,
}
impl Default for SleepClock {
    fn default() -> Self {
        Self {
            mono: Instant::now(),
            wall: SystemTime::now(),
        }
    }
}
impl SleepClock {
    pub fn sample(&mut self, mono: Instant, wall: SystemTime) -> bool {
        let elapsed = mono.saturating_duration_since(self.mono);
        let wall_elapsed = wall.duration_since(self.wall).unwrap_or_default();
        self.mono = mono;
        self.wall = wall;
        wall_elapsed.saturating_sub(elapsed) > Duration::from_secs(30)
    }
}

/// A resume gates all new turns until two probes succeed or two minutes elapse.
#[derive(Default)]
pub struct NetworkGate {
    resumed: Option<Instant>,
    successes: u8,
    ready: bool,
}
impl NetworkGate {
    pub fn resume(&mut self, now: Instant) {
        self.resumed = Some(now);
        self.successes = 0;
        self.ready = false;
    }
    pub fn blocked(&self, now: Instant) -> bool {
        self.resumed
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(120))
            && !self.ready
    }
    pub fn recent_resume(&self, now: Instant) -> bool {
        self.resumed
            .is_some_and(|at| now.saturating_duration_since(at) <= Duration::from_secs(120))
    }
    pub fn probe(&mut self, success: bool) {
        self.successes = if success {
            self.successes.saturating_add(1)
        } else {
            0
        };
        self.ready = self.successes >= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Fake(Vec<bool>);
    impl Inhibitor for Fake {
        fn set(&mut self, active: bool) -> Result<(), String> {
            self.0.push(active);
            Ok(())
        }
    }
    #[test]
    fn keep_awake_acquires_only_for_qualifying_work_and_releases_on_each_edge() {
        let mut awake = Awake::<Fake>::default();
        awake.update(KeepAwake::Off, true, Some(true));
        awake.update(KeepAwake::OnAcPower, true, None);
        awake.update(KeepAwake::OnAcPower, true, Some(true));
        awake.update(KeepAwake::OnAcPower, true, Some(true));
        awake.update(KeepAwake::OnAcPower, true, Some(false));
        awake.update(KeepAwake::WhileRunning, true, None);
        awake.update(KeepAwake::WhileRunning, false, None);
        assert_eq!(awake.os.0, [true, false, true, false]);
    }
    #[test]
    fn fake_clock_and_probe_gate_resume_until_two_consecutive_successes_or_timeout() {
        let now = Instant::now();
        let mut gate = NetworkGate::default();
        assert!(!gate.blocked(now));
        gate.resume(now);
        for probe in [true, false, true] {
            gate.probe(probe);
            assert!(gate.blocked(now + Duration::from_secs(2)));
        }
        gate.probe(true);
        assert!(!gate.blocked(now + Duration::from_secs(4)));
        gate.resume(now);
        assert!(gate.blocked(now + Duration::from_secs(119)));
        assert!(!gate.blocked(now + Duration::from_secs(120)));
        assert!(!gate.recent_resume(now + Duration::from_secs(121)));
    }
    #[test]
    fn sleep_clock_and_idle_threshold() {
        let mut clock = SleepClock::default();
        let mono = clock.mono;
        let wall = clock.wall;
        assert!(!clock.sample(mono + Duration::from_secs(5), wall + Duration::from_secs(5)));
        assert!(clock.sample(
            mono + Duration::from_secs(6),
            wall + Duration::from_secs(100)
        ));
        assert!(!away(Some(AWAY_AFTER - Duration::from_secs(1))));
        assert!(away(Some(AWAY_AFTER)));
        assert!(!away(None));
    }
}
