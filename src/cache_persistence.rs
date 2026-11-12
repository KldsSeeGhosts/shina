//! Port of KSG client-runtime/state/cachePersistence.ts.
//! Producers replace a single queued value. Encoding and atomic-file durability
//! run on the consumer, after 500 ms initially and at most once per ten seconds.
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Pending {
    value: Option<Value>,
    closed: bool,
}

pub struct Writer {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    pub fn new(path: PathBuf, report: impl Fn(String) + Send + 'static) -> Self {
        Self::spawn(
            move |value| {
                let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
                crate::store::write_atomic(&path, &bytes).map_err(|e| e.to_string())
            },
            report,
            Duration::from_millis(500),
            Duration::from_secs(10),
        )
    }

    fn spawn(
        mut persist: impl FnMut(&Value) -> Result<(), String> + Send + 'static,
        report: impl Fn(String) + Send + 'static,
        initial: Duration,
        interval: Duration,
    ) -> Self {
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let queue = pending.clone();
        let worker = std::thread::spawn(move || {
            let (lock, changed) = &*queue;
            let mut deadline = Instant::now() + initial;
            let mut last_error = None;
            loop {
                let mut pending = lock.lock().expect("cache queue");
                while pending.value.is_none() && !pending.closed {
                    pending = changed.wait(pending).expect("cache queue");
                }
                if pending.closed && pending.value.is_none() {
                    break;
                }
                while !pending.closed && Instant::now() < deadline {
                    pending = changed
                        .wait_timeout(pending, deadline.saturating_duration_since(Instant::now()))
                        .expect("cache queue")
                        .0;
                }
                let value = pending.value.take().expect("queued cache");
                drop(pending);
                match persist(&value) {
                    Ok(()) => last_error = None,
                    Err(error) => {
                        if last_error.as_ref() != Some(&error) {
                            report(format!("Could not cache the remote view: {error}"));
                        }
                        last_error = Some(error);
                        let mut pending = lock.lock().expect("cache queue");
                        // Never replace a newer queued view with a failed older one.
                        if !pending.closed && pending.value.is_none() {
                            pending.value = Some(value);
                        }
                    }
                }
                deadline = Instant::now() + interval;
            }
        });
        Self {
            pending,
            worker: Some(worker),
        }
    }

    pub fn submit(&self, value: Value) {
        let (lock, changed) = &*self.pending;
        lock.lock().expect("cache queue").value = Some(value);
        changed.notify_one();
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let (lock, changed) = &*self.pending;
        lock.lock().expect("cache queue").closed = true;
        changed.notify_one();
        // Unlike a browser tab, a native client can flush its final cached view.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    #[test]
    fn coalesces_inflight_updates_off_the_producer_and_flushes_latest_on_exit() {
        let (started, receive) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = writes.clone();
        let producer = std::thread::current().id();
        let mut first = true;
        let writer = Writer::spawn(
            move |value| {
                assert_ne!(std::thread::current().id(), producer);
                if first {
                    first = false;
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                }
                recorded.lock().unwrap().push(value.clone());
                Ok(())
            },
            |_| panic!("unexpected cache error"),
            Duration::ZERO,
            Duration::from_secs(10),
        );
        writer.submit(json!({"seq":1}));
        receive.recv_timeout(Duration::from_secs(1)).unwrap();
        for seq in 2..=1000 {
            writer.submit(json!({"seq":seq}));
        }
        assert!(writes.lock().unwrap().is_empty());
        release.send(()).unwrap();
        let start = Instant::now();
        drop(writer);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(
            *writes.lock().unwrap(),
            [json!({"seq":1}), json!({"seq":1000})]
        );
    }

    #[test]
    fn failure_reports_once_and_does_not_overwrite_a_newer_view() {
        let (failure, reported) = mpsc::channel();
        let saved = Arc::new(Mutex::new(Value::Null));
        let recorded = saved.clone();
        let mut first = true;
        let writer = Writer::spawn(
            move |value| {
                if first {
                    first = false;
                    return Err("fixture failure".into());
                }
                *recorded.lock().unwrap() = value.clone();
                Ok(())
            },
            move |error| failure.send(error).unwrap(),
            Duration::ZERO,
            Duration::from_secs(10),
        );
        writer.submit(json!({"seq":1}));
        assert!(
            reported
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .contains("fixture failure")
        );
        writer.submit(json!({"seq":2}));
        drop(writer);
        assert_eq!(*saved.lock().unwrap(), json!({"seq":2}));
        assert!(reported.try_recv().is_err());
    }

    #[test]
    fn file_cache_is_versioned_and_atomically_readable_after_flush() {
        let directory = crate::store::directory().unwrap().join("cache-test");
        let file = directory.join("view.json");
        let writer = Writer::new(file.clone(), |error| panic!("{error}"));
        writer.submit(json!({"schemaVersion":1,"seq":99,"state":{"text":"latest"}}));
        drop(writer);
        assert_eq!(crate::store::load(&file).unwrap().unwrap()["seq"], 99);
        assert_eq!(
            crate::store::load(&file).unwrap().unwrap()["schemaVersion"],
            1
        );
    }
}
