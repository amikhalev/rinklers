//! Contains various utilites that are used in the rest of the program

use std::cmp::{PartialOrd, Ordering};
use std::time::Duration;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Condvar, Mutex, MutexGuard, LockResult, PoisonError};
use std::ops::{Deref, DerefMut};
use std::fmt;

/// Represents a time to wait for when waiting for an event to occur.
#[derive(Clone, PartialEq, Eq)]
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

impl fmt::Debug for WaitPeriod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::WaitPeriod::*;
        match *self {
            Wait => "Wait".fmt(f),
            AtMost(ref dur) => write!(f, "AtMost({})", duration_string(dur)),
            None => "None".fmt(f),
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
/// Gets a human-readable string representation of a `chrono::Duration`
pub fn chrono_duration_string(dur: &::chrono::Duration) -> String {
    if dur.is_zero() {
        return "0".into();
    }

    let (dur, neg): (::chrono::Duration, bool) = {
        if dur < &::chrono::Duration::zero() {
            (-(*dur), true)
        } else {
            (*dur, false)
        }
    };

    let weeks = dur.num_weeks();
    let days = dur.num_days() % 7;
    let hours = dur.num_hours() % 24;
    let minutes = dur.num_minutes() % 60;
    let secs = dur.num_seconds() % 60;
    let millis = dur.num_milliseconds() % 1000;
    let micros = dur.num_microseconds();
    let nanos = dur.num_nanoseconds();

    let mut s = String::new();
    if neg {
        s.push_str("-")
    }
    if weeks > 0 {
        s.push_str(&(weeks.to_string() + "w"))
    }
    if days > 0 {
        s.push_str(&(days.to_string() + "d"))
    }
    if hours > 0 {
        s.push_str(&(hours.to_string() + "h"))
    }
    if minutes > 0 {
        s.push_str(&(minutes.to_string() + "m"))
    }
    if secs > 0 {
        s.push_str(&(secs.to_string() + "s"))
    }
    if millis > 0 {
        s.push_str(&(millis.to_string() + "ms"))
    }
    if let Some(micros) = micros {
        let micros = micros % 1000;
        if micros > 0 {
            s.push_str(&(micros.to_string() + "us"))
        }
    }
    if let Some(nanos) = nanos {
        let nanos = nanos % 1000;
        if nanos > 0 {
            s.push_str(&(nanos.to_string() + "ns"))
        }
    }
    s
}

/// Gets a string representation of a `std::time::Duration`
pub fn duration_string(duration: &::std::time::Duration) -> String {
    chrono_duration_string(&::chrono::Duration::from_std(*duration).unwrap())
}

/// A guard returned by
/// [LockCondvarGuard.lock_condvar](trait.LockCondvarGuard.html#fn.lock_condvar).
/// It `Deref`s and `DerefMut`s to the underlying `MutexGuard`. It notifies on the `Condvar` when
/// it is `Drop`ed.
pub struct CondvarGuard<'a, T>
    where T: 'a
{
    mutex_guard: MutexGuard<'a, T>,
    condvar: &'a Condvar,
}

impl<'mutex, T> Deref for CondvarGuard<'mutex, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.mutex_guard.deref()
    }
}

impl<'mutex, T> DerefMut for CondvarGuard<'mutex, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.mutex_guard.deref_mut()
    }
}

impl<'a, T> Drop for CondvarGuard<'a, T> {
    fn drop(&mut self) {
        self.condvar.notify_one();
        // this should immediately drop self and self.mutex_guard, which will unlock the mutex
    }
}

/// For objects that can be locked with a `Condvar`
pub trait LockCondvarGuard<T> {
    /// Locks `self`, notifying the `Convar` when the returned `CondvarGuard` is dropped and
    /// unlocks `self`
    fn lock_condvar<'a>(&'a self, condvar: &'a Condvar) -> CondvarGuard<'a, T>;
}

impl<T> LockCondvarGuard<T> for Mutex<T> {
    fn lock_condvar<'a>(&'a self, condvar: &'a Condvar) -> CondvarGuard<'a, T> {
        let guard = self.lock().unwrap();
        CondvarGuard {
            mutex_guard: guard,
            condvar: condvar,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_chrono_duration_string() {
        use ::chrono::Duration;
        let cases: Vec<(Duration, &str)> = vec![
            (Duration::seconds(1), "1s"),
            (Duration::seconds(150), "2m30s"),
            (Duration::weeks(1), "1w"),
            (Duration::days(1), "1d"),
            (Duration::hours(1), "1h"),
            (Duration::minutes(1), "1m"),
            (Duration::milliseconds(1), "1ms"),
            (Duration::microseconds(1), "1us"),
            (Duration::nanoseconds(1), "1ns"),
            (Duration::seconds(31449599) + Duration::nanoseconds(999999999),
            "51w6d23h59m59s999ms999us999ns"),
        ];

        for (dur, expected_string) in cases {
            let expected_string = expected_string.to_string();
            let string = chrono_duration_string(&dur);
            assert_eq!(string, expected_string);
        }
    }
}
