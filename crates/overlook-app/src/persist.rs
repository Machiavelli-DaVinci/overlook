//! Writes the UI thread hands off. One background thread applies them in order, so the
//! window never waits for the disk or for a sync's write, and the last value always wins.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

fn queue() -> &'static Mutex<Sender<Job>> {
    static QUEUE: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let _ = std::thread::Builder::new().name("overlook-persist".into()).spawn(move || {
            for job in rx {
                job();
            }
        });
        Mutex::new(tx)
    })
}

/// Runs `job` on the background writer, after every job queued before it.
pub fn later(job: impl FnOnce() + Send + 'static) {
    if queue().lock().unwrap().send(Box::new(job)).is_err() {
        tracing::warn!("background writer is gone; a setting was not saved");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn jobs_run_in_order() {
        let (tx, rx) = std::sync::mpsc::channel();
        for i in 0..50 {
            let tx = tx.clone();
            super::later(move || tx.send(i).unwrap());
        }
        let got: Vec<i32> = (0..50).map(|_| rx.recv().unwrap()).collect();
        assert_eq!(got, (0..50).collect::<Vec<_>>());
    }
}
