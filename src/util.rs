//! Contains various utilites that are used in the rest of the program

use chrono;
use time;
use std::cmp::{PartialOrd, Ordering};
use std::time::Duration;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Condvar, Mutex, MutexGuard, LockResult, PoisonError, Arc};
use std::ops::{Deref, DerefMut};
use std::fmt;
use serde::{de, Deserialize};

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
            match receiver.recv_timeout(*dur) {
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
                condvar.wait_timeout(guard, *dur)
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

quick_error! {
    /// An error that can occur when converting a `String` to a `Duration`
    #[derive(Debug)]
    pub enum DurationFromStrError {
        /// There was an error converting a `chrono::Duration` to a `std::time::Duration`
        DurationToStd(err: time::OutOfRangeError) {
            description("chrono::Duration out of range of std::time::Duration")
            from()
        }
        /// There was an unexpected character when parsing a Duration
        UnexpectedChar(c: char) {
            description("There was an unexpected character when parsing a Duration")
            display("expected number, whitespace or postfix. found {}", c)
        }
    }
}

/// Converts a `&str` to a `chrono::Duration` in a format where
pub fn chrono_duration_from_str(s: &str) -> Result<chrono::Duration, DurationFromStrError> {
    let mut dur = chrono::Duration::zero();
    let mut num: i64 = 0;

    let mut iter = s.chars()
        .flat_map(|c| c.to_lowercase())
        .peekable();
    while let Some(c) = iter.next() {
        if c.is_whitespace() {
            continue;
        }
        if let Some(digit) = c.to_digit(10) {
            num = num * 10 + digit as i64;
            continue;
        }
        match (c, iter.peek().cloned()) {
            ('w', _) => dur = dur + chrono::Duration::weeks(num),
            ('d', _) => dur = dur + chrono::Duration::days(num),
            ('h', _) => dur = dur + chrono::Duration::hours(num),
            ('s', _) => dur = dur + chrono::Duration::seconds(num),
            ('m', Some('s')) => dur = dur + chrono::Duration::milliseconds(num),
            ('m', _) => dur = dur + chrono::Duration::minutes(num),
            ('u', Some('s')) => dur = dur + chrono::Duration::microseconds(num),
            ('n', Some('s')) => dur = dur + chrono::Duration::nanoseconds(num),
            _ => return Err(DurationFromStrError::UnexpectedChar(c)),
        }
        num = 0;
    }

    Ok(dur)
}

/// Converts a `&str` to a `std::time::Duration`
pub fn duration_from_str(s: &str) -> Result<Duration, DurationFromStrError> {
    chrono_duration_from_str(s).and_then(|dur| {
        dur.to_std()
            .map_err(|err| err.into())
    })
}

/// Deserializes a `std::time::Duration`. Converts it to a string and then uses
/// [`duration_from_str`](#fn.duration_from_str).
pub fn deserialize_duration<D>(d: &mut D) -> Result<Duration, D::Error>
    where D: de::Deserializer
{
    let s = try!(String::deserialize(d));
    duration_from_str(&s).map_err(|err| de::Error::custom(format!("{}", err)))
}

/// Gets a string representation of a `std::time::Duration`
pub fn duration_string(duration: &::std::time::Duration) -> String {
    chrono_duration_string(&::chrono::Duration::from_std(*duration).unwrap())
}

/// A guard returned by
/// [`LockCondvarGuard.lock_condvar`](trait.LockCondvarGuard.html#fn.lock_condvar).
/// It `Deref`s and `DerefMut`s to the underlying `MutexGuard`. It notifies on the `Condvar` when
/// it is `Drop`ed.
pub struct CondvarGuard<'a, T>
    where T: 'a
{
    mutex_guard: MutexGuard<'a, T>,
    condvar: &'a Condvar,
}

impl<'a, T> CondvarGuard<'a, T> {
    fn new(mutex_guard: MutexGuard<'a, T>, condvar: &'a Condvar) -> Self {
        CondvarGuard {
            mutex_guard: mutex_guard,
            condvar: condvar,
        }
    }
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
    fn lock_condvar<'a>(&'a self, condvar: &'a Condvar) -> LockResult<CondvarGuard<'a, T>>;
}

impl<T> LockCondvarGuard<T> for Mutex<T> {
    fn lock_condvar<'a>(&'a self, condvar: &'a Condvar) -> LockResult<CondvarGuard<'a, T>> {
        let guard = self.lock();
        guard.map(|guard| CondvarGuard::new(guard, condvar))
            .map_err(|poison_err| {
                PoisonError::new(CondvarGuard::new(poison_err.into_inner(), condvar))
            })
    }
}

/// The state for a generic "runner" object. Specifically, this stores multi thread state in with a
/// `Mutex`, that when updated will notify a thread using a `Condvar`.
pub struct RunnerState<D> {
    data: Mutex<D>,
    condvar: Condvar,
}

impl<D> RunnerState<D> {
    /// Creates a new `RunnerState` containing the specified data
    pub fn new(data: D) -> Self {
        RunnerState {
            data: Mutex::new(data),
            condvar: Condvar::new(),
        }
    }

    /// Creates a new `RunnerState` in a `Box`
    pub fn new_box(data: D) -> Box<Self> {
        Box::new(Self::new(data))
    }

    /// Creates a new `RunnerState` in an `Arc`
    pub fn new_arc(data: D) -> Arc<Self> {
        Arc::new(Self::new(data))
    }

    /// Begins an update to the `RunnerState`, where changes to the underlying data can be made.
    /// When the `CondvarGuard` is dropped, it will notify the runner thread of the update.
    pub fn update(&self) -> CondvarGuard<D> {
        self.data.lock_condvar(&self.condvar).unwrap()
    }

    /// Notifies the runner thread of an update
    pub fn notify_update(&self) {
        self.condvar.notify_one();
    }

    /// Waits for the state to update. Returns a `MutexGuard` on the data stored in the state.
    pub fn wait_update(&self) -> LockResult<MutexGuard<D>> {
        let guard = try!(self.data.lock());
        self.condvar.wait(guard)
    }

    /// Waits for the state to update for `period`. Returns a `MutexGuard` on the data stored in
    /// the state.
    /// See [WaitPeriod](enum.WaitPeriod.html)
    pub fn wait_update_for_period(&self, period: &WaitPeriod) -> LockResult<MutexGuard<D>> {
        wait_condvar(&self.condvar, &self.data, period)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_chrono_duration_string() {
        use ::chrono::Duration;
        let cases: Vec<(Duration, &str)> = vec![(Duration::seconds(1), "1s"),
                                                (Duration::seconds(150), "2m30s"),
                                                (Duration::weeks(1), "1w"),
                                                (Duration::days(1), "1d"),
                                                (Duration::hours(1), "1h"),
                                                (Duration::minutes(1), "1m"),
                                                (Duration::milliseconds(1), "1ms"),
                                                (Duration::microseconds(1), "1us"),
                                                (Duration::nanoseconds(1), "1ns"),
                                                (Duration::seconds(31449599) +
                                                 Duration::nanoseconds(999999999),
                                                 "51w6d23h59m59s999ms999us999ns")];

        for &(ref dur, ref expected_string) in &cases {
            let expected_string = expected_string.to_string();
            let string = chrono_duration_string(dur);
            assert_eq!(string, expected_string);
        }

        for &(ref expected_dur, ref string) in &cases {
            let dur: Duration = chrono_duration_from_str(string).unwrap();
            assert_eq!(dur, *expected_dur);
        }
    }
}
