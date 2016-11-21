//! Contains methods for waiting for periods of time for events to happen

use std::cmp::{PartialOrd, Ordering};
use std::time::Duration;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Condvar, Mutex, MutexGuard, LockResult, PoisonError};

/// Represents a time to wait for when waiting for an event to occur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitPeriod {
    /// Just wait for the next occurence of the event
    Wait,
    /// Wait at most for the specified duration (wait for a timeout)
    AtMost(Duration),
    /// Don't wait at all and return immediately
    None,
}

impl PartialOrd for WaitPeriod {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WaitPeriod {
    fn cmp(&self, other: &Self) -> Ordering {
        use self::WaitPeriod::*;
        match (self.clone(), other.clone()) {
            (Wait, Wait) | (None, None) => Ordering::Equal,
            (Wait, _) | (_, None) => Ordering::Greater,
            (_, Wait) | (None, _) => Ordering::Less,
            (AtMost(ref dur1), AtMost(ref dur2)) => dur1.cmp(dur2),
        }
    }
}

/// Receives from the specified `Receiver`, while also waiting for whatever amount `wait` is.
///
/// Returns None if `wait` was None or if the timeout was reached. Returns Some if a message was
/// received. Panics if the receive failed for an unspecified reason.
pub fn wait_receiver<T>(receiver: &Receiver<T>, period: &WaitPeriod) -> Option<T> {
    use self::WaitPeriod::*;
    match *period {
        Wait => Some(receiver.recv().unwrap()),
        AtMost(ref dur) => {
            match receiver.recv_timeout(dur.clone()) {
                Ok(recv) => Some(recv),
                Err(RecvTimeoutError::Timeout) => Option::None,
                e @ Err(_) => {
                    e.unwrap();
                    unreachable!()
                }
            }
        }
        None => Option::None,
    }
}

/// Waits on `condvar` for `period`, returning a `MutexGuard` for `mutex` if successful
pub fn wait_condvar<'a, T>(condvar: &Condvar,
                           mutex: &'a Mutex<T>,
                           period: &WaitPeriod)
                           -> LockResult<MutexGuard<'a, T>> {
    use self::WaitPeriod::*;
    let lock = mutex.lock();
    match *period {
        Wait => lock.and_then(|guard| condvar.wait(guard)),
        AtMost(ref dur) => {
            lock.and_then(|guard| {
                condvar.wait_timeout(guard, dur.clone())
                    .map(|res| res.0)
                    .map_err(|err| PoisonError::new(err.into_inner().0))
            })
        }
        None => lock,
    }
}
