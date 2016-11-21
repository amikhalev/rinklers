//! ScheduleRunner for executing events based on Schedules

use std::thread::{self, JoinHandle};
use std::fmt;
use std::cmp;
use chrono::{DateTime, Local, Duration as CDuration};
use schedule::Schedule;

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

use std::sync::{Mutex, Condvar};
use std::marker::PhantomData;

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

impl<E: Executor> fmt::Debug for SchedRun<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "SchedRun {{ sched: {:?}, data: .., next_run: {:?} }}",
               self.sched,
               self.next_run)
    }
}

#[derive(Debug)]
struct RunnerData<E>
    where E: Executor
{
    quit: bool,
    runs: Vec<SchedRun<E>>,
}

impl<E: Executor> RunnerData<E> {
    fn new() -> Self {
        RunnerData {
            quit: false,
            runs: Vec::new(),
        }
    }
}

struct RunnerState<E>
    where E: Executor
{
    data: Mutex<RunnerData<E>>,
    condvar: Condvar,
}

impl<E: Executor> RunnerState<E> {
    fn new() -> Self {
        RunnerState {
            data: Mutex::new(RunnerData::new()),
            condvar: Condvar::new(),
        }
    }

    fn update<F>(&self, f: F)
        where F: FnOnce(&mut RunnerData<E>)
    {
        {
            let mut lock = self.data.lock().unwrap();
            f(&mut *lock);
        }
        self.condvar.notify_one();
    }
}

/// Keeps track of [Schedule](struct.Schedule.html)s and executes events when they are triggered by
/// the schedules
pub struct ScheduleRunner<E>
    where E: Executor + 'static
{
    state: Box<RunnerState<E>>,
    join_handle: Option<JoinHandle<()>>,
    _phantom_data: PhantomData<E>,
}

impl<E> ScheduleRunner<E>
    where E: Executor
{
    /// Starts a new `ScheduleRunner` using the specified `Executor`
    /// Note: if the `ScheduleRunner` is dropped without calling [stop](#method.stop), the thread
    /// associated with it will be detached rather than stopped.
    pub fn start_new(executor: E) -> ScheduleRunner<E> {
        let mut runner = ScheduleRunner {
            state: Box::new(RunnerState::new()),
            join_handle: None,
            _phantom_data: PhantomData::default(),
        };
        runner.join_handle = unsafe {
            // this should be ok, because all of the referenced data is stored in the same object
            // as the JoinHandle for the thread, and if it gets dropped the Drop impl should join
            // the thread.
            let state: &'static RunnerState<E> =
                ::std::mem::transmute(&*runner.state as &RunnerState<E>);
            Some(thread::spawn(move || Self::run(state, executor)))
        };
        runner
    }

    /// Adds the specified `schedule`, running the executor with the specified `data` whenever the
    /// schedule triggers it.
    pub fn schedule(&self, schedule: Schedule, sched_data: E::Data) {
        self.state.update(move |data| {
            data.runs.push(SchedRun::new(schedule, sched_data));
        });
    }

    /// Stops the `ScheduleRunner`, waiting for the thread to exit
    pub fn stop(self) {
        self.state.update(|data| {
            data.quit = true;
        });
    }

    fn run(state: &RunnerState<E>, executor: E) {
        let data = &state.data;
        let condvar = &state.condvar;
        use wait_for::{WaitPeriod, wait_condvar};
        let mut wait_period = WaitPeriod::Wait;
        loop {
            let mut data_lock = wait_condvar(&condvar, &data, &wait_period).unwrap();

            // trace!("ScheduleRunner state {:?}", *data_lock);
            if data_lock.quit {
                debug!("quitting ScheduleRunner");
                return;
            }

            wait_period = WaitPeriod::Wait;
            for run in &mut data_lock.runs {
                if let Some(left) = run.till_run() {
                    if left <= CDuration::zero() {
                        // if the time left is less than or equal to zero, it is time for the
                        // event to run
                        executor.execute(&run.data);
                        run.update_next_run();
                        wait_period = WaitPeriod::None;
                    } else {
                        // sleep until it is time to run
                        let left = left.to_std().unwrap();
                        wait_period = cmp::min(wait_period, WaitPeriod::AtMost(left));
                    }
                }
            }
            // only keep runs that will occur at some point in the future
            data_lock.runs.retain(|run| run.next_run.is_some());
        }
    }
}

impl<E: Executor> Drop for ScheduleRunner<E> {
    fn drop(&mut self) {
        // waits for the thread to finish, because it references data stored in self, so it cannot
        // outlive self
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().unwrap();
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_schedule_runner() {
        use schedule::{DateTimeBound, Schedule};
        use chrono::{Local, Datelike, Duration as CDuration};
        use std::time::Duration;
        use std::sync::mpsc::{Receiver, channel};
        let schedule_runner = ScheduleRunner::start_new(FnExecutor);

        let delay_time = CDuration::milliseconds(25);
        let now = Local::now();
        let run_time = now.time() + delay_time;
        let today = now.weekday();
        let schedule = Schedule::new(vec![run_time],
                                     vec![today],
                                     DateTimeBound::None,
                                     DateTimeBound::None);

        let mut rxs: Vec<Receiver<()>> = Vec::new();
        for _ in 0..10 {
            let (tx, rx) = channel::<()>();
            schedule_runner.schedule(schedule.clone(),
                                     Box::new(move || {
                                         tx.send(()).unwrap();
                                     }));
            rxs.push(rx);
        }
        for rx in &rxs {
            rx.recv_timeout(Duration::from_millis(50)).ok().expect("schedule did not run in time");
        }
        for rx in &rxs {
            rx.recv_timeout(Duration::from_millis(50))
                .err()
                .expect("schedule ran again when it should not have");
        }

        schedule_runner.stop();
    }
}
