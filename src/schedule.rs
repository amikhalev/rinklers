//! Scheduling for events to run at certain intervals in the future

use std::collections::{BTreeSet, HashSet};
use std::iter::FromIterator;
use std::cmp;
use std::time::Duration;
use chrono::{DateTime, Date, Datelike, TimeZone, Local, NaiveTime, Weekday, Duration as CDuration};

type TimeSet = BTreeSet<NaiveTime>;
type WeekdaySet = HashSet<Weekday>;

/// Represents the different types of date-time bounds that can be on a schedule
#[derive(Debug)]
pub enum DateTimeBound {
    /// There is no bound (ie. the Schedule extends with no limit)
    None,
    /// There is a bound that repeats every year (ie. the year is set to the current year)
    Yearly(DateTime<Local>),
    /// There is a definite bound on the schedule
    Definite(DateTime<Local>),
}

impl Default for DateTimeBound {
    fn default() -> DateTimeBound {
        DateTimeBound::None
    }
}

impl DateTimeBound {
    /// Resolves this bound into an optional `DateTime`. If there is no bound or the bound could
    /// not be resolved, None is returned.
    /// 
    /// `date` is the reference that is used to resolve a `Yearly` bound.
    pub fn resolve_from(&self, date: &DateTime<Local>) -> Option<DateTime<Local>> {
        match *self {
            DateTimeBound::None => None,
            DateTimeBound::Yearly(date_time) => date_time.with_year(date.year()),
            DateTimeBound::Definite(date_time) => Some(date_time),
        }
    }

    /// Resolves this bound with the current date and time
    /// 
    /// See [resolve_from](#method.resolve_from)
    pub fn resolve(&self) -> Option<DateTime<Local>> {
        self.resolve_from(&Local::now())
    }
}

/// A schedule that determines when an event will occur.
#[derive(Default, Debug)]
pub struct Schedule {
    times: TimeSet,
    weekdays: WeekdaySet,
    from: DateTimeBound,
    to: DateTimeBound,
}

/// Gets the next date matching the `weekday` after `date`
fn next_weekday<Tz: TimeZone>(date: &Date<Tz>, weekday: &Weekday) -> Date<Tz> {
    let mut date = date.clone();
    while &date.weekday() != weekday {
        date = date.succ();
    }
    date
}

impl Schedule {
    /// Creates a new Schedule.
    ///
    /// `times` is the times of day the event will be run. `weekdays` is the set of days of week
    /// the event will be run. `from` and `to` are restrictions on the end and beginning of event
    /// runs, respectively.
    pub fn new<T, W>(times: T, weekdays: W, from: DateTimeBound, to: DateTimeBound) -> Schedule
        where T: IntoIterator<Item = NaiveTime>,
              W: IntoIterator<Item = Weekday>
    {
        Schedule {
            times: TimeSet::from_iter(times),
            weekdays: WeekdaySet::from_iter(weekdays),
            from: from,
            to: to,
        }
    }

    /// Gets the next `DateTime` the event should run after `reference`
    /// 
    /// Returns `None` if the event will never run after `reference` (ie. must be a `from` bound)
    pub fn next_run_after(&self, reference: &DateTime<Local>) -> Option<DateTime<Local>> {
        let to = self.to.resolve_from(reference);
        let from = match (self.from.resolve_from(reference), to) {
            (Some(from), Some(to)) if from < to => from.with_year(from.year() + 1),
            (from, _) => from,
        };
        let reference = match from {
            Some(from) if &from > reference => from,
            _ => reference.clone(),
        };
        let mut candidate: Option<DateTime<Local>> = None;
        for weekday in self.weekdays.iter() {
            for time in self.times.iter() {
                let date = next_weekday(&reference.date(), weekday)
                    .and_time(time.clone())
                    .map(|date| if date < reference {
                        date + CDuration::weeks(1)
                    } else {
                        date
                    });
                let date = match (date, to) {
                    (Some(date), Some(to)) if date > to => None,
                    (date, _) => date,
                };
                candidate = match (candidate, date) {
                    // return whichever is first if there are 2 candidates
                    (Some(d1), Some(d2)) => Some(cmp::min(d1, d2)),
                    // otherwise return whichever isn't None (or None if both are)
                    (o1, o2) => o1.or(o2),
                }
            }
        }
        candidate
    }

    /// Gets the next run after the current time
    /// 
    /// See [next_run_after](#method.next_run_after)
    pub fn next_run(&self) -> Option<DateTime<Local>> {
        self.next_run_after(&Local::now())
    }
}

use std::sync::mpsc::{channel, Sender, Receiver, RecvTimeoutError};
use std::thread;
use std::fmt;

/// A time to sleep for using a method receive with a timeout
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sleep {
    /// Just wait for the next received message (use `Receiver.recv`)
    WaitForRecv,
    /// Wait for the specified duration (use `Receiver.recv_timeout`)
    AtMost(Duration),
    /// Don't wait at all and return None immediately
    None,
}

/// Receives from the specified `Receiver`, while also sleeping for whatever amount `sleep` is.
/// 
/// Returns None if Sleep was None or if the timeout was reached. Returns Some if a message was
/// received.
pub fn sleep_recv<T>(sleep: &Sleep, rx: &Receiver<T>) -> Option<T> {
    use self::Sleep::*;
    match sleep {
        &WaitForRecv => Some(rx.recv().unwrap()),
        &AtMost(dur) => {
            match rx.recv_timeout(dur) {
                Ok(recv) => Some(recv),
                Err(RecvTimeoutError::Timeout) => Option::None,
                e @ Err(_) => {
                    e.unwrap();
                    unreachable!()
                }
            }
        }
        &None => Option::None,
    }
}

use std::cmp::{PartialOrd, Ordering};

impl PartialOrd for Sleep {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Sleep {
    fn cmp(&self, other: &Self) -> Ordering {
        use self::Sleep::*;
        match (self, other) {
            (&WaitForRecv, &WaitForRecv) |
            (&None, &None) => Ordering::Equal,
            (&WaitForRecv, _) |
            (_, &None) => Ordering::Greater,
            (_, &WaitForRecv) |
            (&None, _) => Ordering::Less,
            (&AtMost(ref dur1), &AtMost(ref dur2)) => dur1.cmp(dur2),
        }
    }
}

/// Executes events triggered by a [ScheduleRunner](struct.ScheduleRunner.html)
pub trait Executor: Send {
    /// The data that is stored by the `ScheduleRunner` when the event is scheduled and that will
    /// be passed to the executor when the event is triggered.
    type Data: Send + 'static;

    /// Executed when an event is triggered
    fn execute(&self, data: &Self::Data);
}

/// An [Executor](trait.Executor.html) that doesn't do anything
pub struct NoopExecutor;

impl Executor for NoopExecutor {
    type Data = ();

    fn execute(&self, _: &Self::Data) {}
}

/// An [Executor](trait.Executor.html) that runs a Fn()
pub struct FnExecutor;

impl Executor for FnExecutor {
    type Data = Box<Fn() + Send>;

    fn execute(&self, data: &Self::Data) {
        data();
    }
}

enum Op<E>
    where E: Executor + 'static
{
    Quit,
    Schedule(Schedule, E::Data),
    Cancel,
}

impl<E> fmt::Debug for Op<E>
    where E: Executor + 'static
{
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &Op::Quit => write!(formatter, "Quit"),
            &Op::Schedule(ref sched, _) => write!(formatter, "Schedule({:?}, Data)", sched),
            &Op::Cancel => write!(formatter, "Cancel"),
        }
    }
}

/// Keeps track of [Schedule](struct.Schedule.html)s and executes events when they are triggered by
/// the schedules
pub struct ScheduleRunner<E>
    where E: Executor + 'static
{
    sender: Sender<Op<E>>,
}

impl<E> ScheduleRunner<E>
    where E: Executor
{
    /// Starts a new `ScheduleRunner` using the specified `Executor`
    pub fn start_new(executor: E) -> ScheduleRunner<E> {
        let (send, recv) = channel();
        thread::spawn(move || Self::run(recv, executor));
        ScheduleRunner { sender: send }
    }

    /// Adds the specified `schedule`, running the executor with the specified `data` whenever the
    /// schedule triggers it.
    pub fn schedule(&self, schedule: Schedule, data: E::Data) {
        self.sender.send(Op::Schedule(schedule, data)).unwrap();
    }

    /// Stops the `ScheduleRunner`
    pub fn stop(self) {
        self.sender.send(Op::Quit).unwrap();
        // drops self
    }

    fn run(recv: Receiver<Op<E>>, executor: E) {
        struct SchedRun<E: Executor> {
            sched: Schedule,
            data: E::Data,
            next_run: Option<DateTime<Local>>,
        }
        impl<E: Executor> SchedRun<E> {
            fn new(sched: Schedule, data: E::Data) -> Self {
                let next_run = sched.next_run();
                SchedRun {
                    sched: sched,
                    data: data,
                    next_run: next_run,
                }
            }

            fn till_run(&self) -> Option<CDuration> {
                self.next_run.map(|next_run| next_run - Local::now())
            }

            fn update_next_run(&mut self) -> Option<DateTime<Local>> {
                self.next_run = self.sched.next_run();
                self.next_run
            }
        }
        let mut runs: Vec<SchedRun<E>> = Vec::new();
        let mut sleep = Sleep::WaitForRecv;
        loop {
            let op = sleep_recv(&sleep, &recv);
            trace!("ScheduleRunner op {:?}", op);
            if let Some(op) = op {
                match op {
                    Op::Quit => {
                        debug!("quitting ScheduleRunner");
                        return;
                    }
                    Op::Schedule(sched, data) => runs.push(SchedRun::new(sched, data)),
                    Op::Cancel => {
                        unimplemented!();
                    }
                }
            }
            sleep = Sleep::WaitForRecv;
            for run in &mut runs {
                if let Some(left) = run.till_run() {
                    if left <= CDuration::zero() {
                        // if the time left is less than or equal to zero, it is time for the
                        // event to run
                        executor.execute(&run.data);
                        run.update_next_run();
                    } else {
                        // otherwise, sleep until it is time to run
                        let left = left.to_std().unwrap();
                        sleep = cmp::min(sleep, Sleep::AtMost(left));
                    }
                }
            }
            runs.retain(|run| run.next_run.is_some()); // remove all runs that will never occur
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_program() {
        use super::{Schedule, DateTimeBound};
        use chrono::{NaiveTime, Weekday, Local, TimeZone};
        let schedule = Schedule::new([NaiveTime::from_hms(10, 30, 0)].iter().map(|t| t.clone()),
                                     [Weekday::Wed].iter().map(|t| t.clone()),
                                     DateTimeBound::None,
                                     DateTimeBound::None);
        let date = schedule.next_run_after(&Local.ymd(2016, 11, 14).and_hms(10, 30, 0));
        assert_eq!(date, Some(Local.ymd(2016, 11, 16).and_hms(10, 30, 0)));
        let date = schedule.next_run_after(&Local.ymd(2016, 11, 16).and_hms(10, 20, 0));
        assert_eq!(date, Some(Local.ymd(2016, 11, 16).and_hms(10, 30, 0)));
        let date = schedule.next_run_after(&Local.ymd(2016, 11, 16).and_hms(10, 40, 0));
        assert_eq!(date, Some(Local.ymd(2016, 11, 23).and_hms(10, 30, 0)));
    }
}
