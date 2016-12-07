//! Contains `SectionRunner`

use std::time::{Duration, Instant};
use std::sync::mpsc::{Sender, Receiver, channel};
use std::thread::{self, JoinHandle};
use std::collections::VecDeque;
use std::fmt;
use section::SectionRef;
use util::duration_string;

/// An operation that can be sent to a `SectionRunner`
#[derive(Debug)]
enum Op {
    Quit,
    QueueRun(SecRun),
}

/// A notification that can be returned from
/// [`run_section`](struct.SectionRunner.html#method.run_section)
pub enum RunNotification {
    /// The section has started running
    Start,
    /// The section has finished running
    Finish,
    /// The section was interrupted while running (the `SectionRunner` was stopped)
    Interrupted,
}

/// A type that can be used to receive notifications about a Section run
pub type RunNotifier = Receiver<RunNotification>;

/// Information about a single run of a section
struct SecRun {
    sec: SectionRef,
    dur: Duration,
    // may be open or closed. will not cause an error if the other side is hung up
    notification_sender: Sender<RunNotification>,
}

impl fmt::Debug for SecRun {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "SecRun {{ sec: {:?}, dur: {} }}",
               self.sec,
               duration_string(&self.dur))
    }
}

/// Runs sections for periods of time
///
/// Runs a thread in the background which manages a queue of section runs. Each section runs for a
/// period of time determined by a `Duration`.
pub struct SectionRunner {
    tx: Sender<Op>,
    // only the first `SectionRunner` returned from `start_new` has the join_handle, every one
    // after that will have `None`
    join_handle: Option<JoinHandle<()>>,
}

impl SectionRunner {
    /// Starts a new `SectionRunner` thread and returns a `SectionRunner` struct for managing it
    pub fn start_new() -> SectionRunner {
        let (tx, rx) = channel::<Op>();
        let join_handle = thread::spawn(move || Self::run(rx));
        SectionRunner {
            tx: tx,
            join_handle: Some(join_handle),
        }
    }

    /// Queues a run for section `section` for a period of time specified by `dur`.
    ///
    /// Returns a `Receiver<[RunNotification](enum.RunNotification.html)>` which can be used to
    /// receive notifications about the progress of the section run. If this `Receiver` is dropped
    /// and closed, it will not cause an error.
    ///
    /// # Notes
    /// Sections are queued to run in the same order that this method is called in
    pub fn run_section(&self, sec: SectionRef, dur: Duration) -> RunNotifier {
        let (send, recv) = channel::<RunNotification>();
        let run = SecRun {
            sec: sec,
            dur: dur,
            notification_sender: send,
        };
        self.tx.send(Op::QueueRun(run)).unwrap();
        recv
    }

    /// Tells the `SectionRunner` to stop and waits until its thread quits
    ///
    /// Any section runs that are queued are discarded, and if a section is currently running it
    /// will be interrupted.
    ///
    /// # Panics
    /// This method will panic if `self` is a cloned copy of the original `SectionRunner`. Only the
    /// `SectionRunner` returned from `start_new` can be used to stop the thread.
    pub fn stop(self) {
        let join_handle = self.join_handle
            .expect("SectionRunner was attempted to be stopped from a cloned copy");
        self.tx.send(Op::Quit).unwrap();
        join_handle.join().unwrap();
    }

    /// Runs the thread which does all of the magic
    fn run(rx: Receiver<Op>) {
        use self::Op::*;
        use util::{WaitPeriod, wait_receiver};
        struct Run {
            sec: SectionRef,
            dur: Duration,
            start_time: Instant,
            notify: Sender<RunNotification>,
        }
        let mut current_run: Option<Run> = None;
        let mut wait_period: WaitPeriod = WaitPeriod::Wait;
        let mut queue: VecDeque<SecRun> = VecDeque::new();
        loop {
            let op = wait_receiver(&rx, &wait_period);
            trace!("SectionRunner op {:?}", op);
            if let Some(op) = op {
                match op {
                    Quit => {
                        if let Some(run) = current_run {
                            debug!("interrupting section {:?}, ran for {}",
                                   run.sec,
                                   duration_string(&run.start_time.elapsed()));
                            run.sec.set_state(false);
                            // if the receiver is closed, it's ok
                            let _ = run.notify.send(RunNotification::Interrupted);
                            for run in queue {
                                let _ = run.notification_sender.send(RunNotification::Interrupted);
                            }
                        }
                        return;
                    }
                    QueueRun(run) => queue.push_back(run),
                }
            }
            current_run = {
                if let Some(run) = current_run {
                    let elapsed = run.start_time.elapsed();
                    if elapsed >= run.dur {
                        trace!("finished running section {:?}, ran for {}",
                               run.sec,
                               duration_string(&elapsed));
                        run.sec.set_state(false);
                        // if the receiver is closed, it's ok
                        let _ = run.notify.send(RunNotification::Finish);
                        wait_period = WaitPeriod::None;
                        None
                    } else {
                        let sleep_dur = run.dur - elapsed;
                        wait_period = WaitPeriod::AtMost(sleep_dur);
                        Some(run)
                    }
                } else if let Some(run) = queue.pop_front() {
                    debug!("running section {:?} for {}",
                           run.sec,
                           duration_string(&run.dur));
                    run.sec.set_state(true);
                    // if the receiver is closed, it's ok
                    let _ = run.notification_sender.send(RunNotification::Start);
                    wait_period = WaitPeriod::AtMost(run.dur);
                    Some(Run {
                        start_time: Instant::now(),
                        sec: run.sec,
                        dur: run.dur,
                        notify: run.notification_sender,
                    })
                } else {
                    wait_period = WaitPeriod::Wait;
                    None
                }
            };
        }
    }
}

impl Clone for SectionRunner {
    /// Clones the `SectionRunner`. The cloned `SectionRunner` can be used to send messages to the
    /// runner thread, but it can not be used to stop the thread. If the first `SectionRunner`
    /// calls `stop`, any operations attempted from other `SectionRunner`s will panic.
    fn clone(&self) -> Self {
        SectionRunner {
            tx: self.tx.clone(),
            join_handle: None,
        }
    }
}
