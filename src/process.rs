//! Owned subprocesses. Dropping the owner terminates its process tree.
use std::{
    io::{self, Read},
    ops::{Deref, DerefMut},
    process::{Child, Command, Output, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// A liveness check only. Never use this to authorize terminating a PID.
pub fn is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, id: u32) -> *mut c_void;
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let alive = unsafe { WaitForSingleObject(handle, 0) } == 258;
        unsafe { CloseHandle(handle) };
        alive
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

pub struct Process {
    child: Child,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl Process {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Assign the job before any child code can create descendants.
            command.creation_flags(0x08000000 | 0x00000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        }
        let child = command.spawn()?;
        #[cfg(windows)]
        let job = match windows::own(&child).and_then(|job| {
            windows::resume(&child)?;
            Ok(job)
        }) {
            Ok(job) => job,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = wait_child(&mut child, Duration::from_secs(1));
                return Err(error);
            }
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            _job: job,
        })
    }

    pub fn terminate(&mut self) {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows::TerminateJobObject(self._job.as_raw_handle(), 1);
            }
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
    }

    pub fn reap(&mut self, timeout: Duration) -> io::Result<()> {
        wait_child(&mut self.child, timeout).map(drop)
    }
}

pub fn wait_child(child: &mut Child, timeout: Duration) -> io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Subprocess did not exit.",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

impl Deref for Process {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for Process {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.terminate();
        // Reap without blocking a UI caller. Normal workers reap explicitly.
        let _ = self.child.try_wait();
    }
}

pub fn join_for<T>(worker: JoinHandle<T>, timeout: Duration) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
    worker.join().ok()
}

/// Bounded output and wall-clock time for registration and other short helper commands.
pub fn output(command: &mut Command, timeout: Duration, limit: usize) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = Process::spawn(command)?;
    let stdout = process.stdout.take().expect("piped stdout");
    let stderr = process.stderr.take().expect("piped stderr");
    let read = move |pipe: Box<dyn Read + Send>| {
        let mut bytes = Vec::new();
        pipe.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    };
    let out = thread::spawn(move || read(Box::new(stdout)));
    let err = thread::spawn(move || read(Box::new(stderr)));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = process.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            process.terminate();
            let _ = process.reap(Duration::from_secs(1));
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Subprocess timed out.",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    process.terminate(); // Descendants must not keep the output pipes open.
    let stdout = join_for(out, Duration::from_secs(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "stdout did not close."))??;
    let stderr = join_for(err, Duration::from_secs(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "stderr did not close."))??;
    if stdout.len() > limit || stderr.len() > limit {
        return Err(io::Error::other("Subprocess output exceeded its limit."));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_processes: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        pub fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, id: u32) -> *mut c_void;
        fn Thread32First(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, id: u32) -> *mut c_void;
        fn ResumeThread(thread: *mut c_void) -> u32;
    }
    #[repr(C)]
    #[derive(Default)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        id: u32,
        owner: u32,
        priority: i32,
        delta: i32,
        flags: u32,
    }

    pub fn resume(child: &Child) -> io::Result<()> {
        let raw = unsafe { CreateToolhelp32Snapshot(4, 0) }; // TH32CS_SNAPTHREAD
        if raw as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry = ThreadEntry {
            size: std::mem::size_of::<ThreadEntry>() as u32,
            ..Default::default()
        };
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while found != 0 {
            if entry.owner == child.id() {
                let raw = unsafe { OpenThread(2, 0, entry.id) }; // THREAD_SUSPEND_RESUME
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        Err(io::Error::other(
            "Could not find the suspended child's main thread.",
        ))
    }
    pub fn own(child: &Child) -> io::Result<OwnedHandle> {
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits = ExtendedLimits::default();
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if unsafe {
            SetInformationJobObject(
                raw,
                9,
                &limits as *const _ as *const c_void,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(raw, child.as_raw_handle()) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(script: &str) -> Command {
        #[cfg(windows)]
        let mut command = Command::new("powershell");
        #[cfg(windows)]
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        #[cfg(unix)]
        let mut command = Command::new("sh");
        #[cfg(unix)]
        command.args(["-c", script]);
        command
    }

    #[test]
    fn bounded_commands_limit_time_and_output() {
        let sleep = if cfg!(windows) {
            "Start-Sleep -Seconds 10"
        } else {
            "sleep 10"
        };
        let start = Instant::now();
        assert!(output(&mut command(sleep), Duration::from_millis(200), 128).is_err());
        assert!(start.elapsed() < Duration::from_secs(4));
        let verbose = if cfg!(windows) {
            "Write-Output ('x' * 2000)"
        } else {
            "printf '%02000d' 0"
        };
        assert!(output(&mut command(verbose), Duration::from_secs(10), 128).is_err());
    }

    #[test]
    fn termination_unblocks_a_writer_when_the_child_never_reads_stdin() {
        use std::io::Write;
        let script = if cfg!(windows) {
            "Start-Sleep -Seconds 60"
        } else {
            "sleep 60"
        };
        let mut command = command(script);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = Process::spawn(&mut command).unwrap();
        let mut stdin = process.stdin.take().unwrap();
        let writer = thread::spawn(move || stdin.write_all(&vec![b'x'; 16 * 1024 * 1024]));
        process.terminate();
        process.reap(Duration::from_secs(2)).unwrap();
        assert!(join_for(writer, Duration::from_secs(2)).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn terminating_a_job_stops_descendants_and_closes_their_pipes() {
        use std::{
            ffi::c_void,
            io::{BufRead, BufReader},
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        };
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, id: u32) -> *mut c_void;
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        }
        let mut command = command(
            "$child = Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60' -PassThru; Write-Output $child.Id; Start-Sleep -Seconds 60",
        );
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        let mut process = Process::spawn(&mut command).unwrap();
        let pipe = process.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            let mut pipe = BufReader::new(pipe);
            let mut line = String::new();
            pipe.read_line(&mut line).unwrap();
            sender.send(line).unwrap();
            let mut tail = String::new();
            pipe.read_to_string(&mut tail).unwrap();
        });
        let id = receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let handle = unsafe { OpenProcess(0x00100000, 0, id) };
        assert!(!handle.is_null());
        let child = unsafe { OwnedHandle::from_raw_handle(handle) };
        process.terminate();
        process.reap(Duration::from_secs(2)).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(child.as_raw_handle(), 2000) },
            0
        );
        assert!(join_for(reader, Duration::from_secs(2)).is_some());
    }
}
