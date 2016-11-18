//! ScheduleRunner for executing events based on Schedules

use std::sync::mpsc::{channel, Sender, Receiver, RecvTimeoutError};
use std::time::Duration;
use std::thread::{self, JoinHandle};
use std::fmt;
use std::cmp;
use chrono::{DateTime, Local, Duration as CDuration};
use schedule::Schedule;

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
    join_handle: JoinHandle<()>,
}

impl<E> ScheduleRunner<E>
    where E: Executor
{
    /// Starts a new `ScheduleRunner` using the specified `Executor`
    /// Note: if the `ScheduleRunner` is dropped without calling [stop](#method.stop), the thread
    /// associated with it will be detached rather than stopped.
    pub fn start_new(executor: E) -> ScheduleRunner<E> {
        let (send, recv) = channel();
        let join_handle = thread::spawn(move || Self::run(recv, executor));
        ScheduleRunner { sender: send, join_handle: join_handle }
    }

    /// Adds the specified `schedule`, running the executor with the specified `data` whenever the
    /// schedule triggers it.
    pub fn schedule(&self, schedule: Schedule, data: E::Data) {
        self.sender.send(Op::Schedule(schedule, data)).unwrap();
    }

    /// Stops the `ScheduleRunner`, waiting for the thread to exit
    pub fn stop(self) {
        self.sender.send(Op::Quit).unwrap();
        self.join_handle.join().unwrap();
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
    use super::*;

    #[test]
    fn test_schedule_runner() {
        use schedule::Schedule;
        use std::time::Duration;
        use std::sync::mpsc::channel;
        let schedule_runner = ScheduleRunner::start_new(FnExecutor);

        let schedule = Schedule::default();

        let (tx, rx) = channel::<()>();
        schedule_runner.schedule(schedule, Box::new(move || { tx.send(()).unwrap(); }));

        rx.recv_timeout(Duration::from_millis(50)).ok().expect("schedule did not run in time");

        schedule_runner.stop();
    }
}
