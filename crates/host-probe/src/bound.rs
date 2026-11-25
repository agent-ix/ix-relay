//! A source read under a time budget (NFR-003, FR-069): the read runs on its own
//! thread, the tick waits at most the budget, and a read still running is not
//! started a second time. A hung source costs the tick its budget, not more.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use crate::ErrorClass;

/// How long one source may delay a tick.
pub const SOURCE_BUDGET: Duration = Duration::from_millis(500);

/// One source's guard against a second concurrent read.
#[derive(Debug, Clone, Default)]
pub struct Slot {
    busy: Arc<AtomicBool>,
}

struct Release(Arc<AtomicBool>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Slot {
    /// Runs `job` for at most `budget`.
    ///
    /// # Errors
    ///
    /// [`ErrorClass::Timeout`] when the budget passes, or when an earlier read
    /// of this slot has not returned; [`ErrorClass::Unreadable`] when no thread
    /// could be started.
    pub fn run<T: Send + 'static>(
        &self,
        budget: Duration,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, ErrorClass> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err(ErrorClass::Timeout);
        }
        let release = Release(Arc::clone(&self.busy));
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("host-probe-source".into())
            .spawn(move || {
                // The slot frees before the result is sent, so a caller that sees the
                // result can start the next read at once.
                let value = job();
                drop(release);
                let _ = tx.send(value);
            })
            .map_err(|_| {
                self.busy.store(false, Ordering::Release);
                ErrorClass::Unreadable
            })?;
        rx.recv_timeout(budget).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => ErrorClass::Timeout,
            // The job panicked: nothing was read.
            mpsc::RecvTimeoutError::Disconnected => ErrorClass::Unreadable,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn a_job_that_finishes_returns_its_value() {
        let slot = Slot::default();
        assert_eq!(slot.run(Duration::from_secs(5), || 7), Ok(7));
        assert_eq!(
            slot.run(Duration::from_secs(5), || 8),
            Ok(8),
            "slot is free again"
        );
    }

    #[test]
    fn a_hung_job_times_out_and_is_not_started_twice() {
        let slot = Slot::default();
        let (release, hold) = mpsc::channel::<()>();
        let started = Instant::now();
        let got = slot.run(Duration::from_millis(50), move || {
            let _ = hold.recv();
        });
        assert_eq!(got, Err(ErrorClass::Timeout));
        assert!(started.elapsed() < Duration::from_secs(5));
        // Still running: the next read is refused at once.
        let again = Instant::now();
        assert_eq!(
            slot.run(Duration::from_secs(5), || 1),
            Err(ErrorClass::Timeout)
        );
        assert!(again.elapsed() < Duration::from_millis(500));
        drop(release);
        // Once it ends the slot frees (poll briefly).
        let mut freed = false;
        for _ in 0..200 {
            if slot.run(Duration::from_secs(1), || 1).is_ok() {
                freed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(freed);
    }

    #[test]
    fn a_panicking_job_is_unreadable() {
        let slot = Slot::default();
        let got: Result<(), _> = slot.run(Duration::from_secs(1), || panic!("boom"));
        assert_eq!(got, Err(ErrorClass::Unreadable));
    }
}
