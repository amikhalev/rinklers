use std::time::{Duration, Instant};
use std::sync::mpsc::{Sender, Receiver, channel, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::collections::VecDeque;
use std::fmt;
use section::Section;

/// Gets a human-readable string representation of a `Duration`
fn duration_str(dur: &Duration) -> String {
    let mut secs = dur.as_secs();
    let mut minutes = secs / 60;
    secs %= 60;
    let hours = minutes / 60;
    minutes %= 60;
    let mut nanos = dur.subsec_nanos();
    let mut micros = nanos / 1000;
    nanos %= 1000;
    let millis = micros / 1000;
    micros %= 1000;

    let mut s = String::new();
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
    if micros > 0 {
        s.push_str(&(micros.to_string() + "us"))
    }
    if nanos > 0 {
        s.push_str(&(nanos.to_string() + "ns"))
    }
    s
}

/// An operation that can be sent to a SectionRunner
#[derive(Debug)]
enum Op {
    Quit,
    QueueRun(SecRun)
}

/// A notification that can be returned from [run_section](struct.SectionRunner.html#method.run_section)
pub enum RunNotification {
    /// The section has started running
    Start,
    /// The section has finished running
    Finish,
    /// The section was interrupted while running (the `SectionRunner` was stopped)
    Interrupted,
}

/// Information about a single run of a section
struct SecRun {
    sec: Arc<Section>,
    dur: Duration,
    // may be open or closed. will not cause an error if the other side is hung up
    notification_sender: Sender<RunNotification>,
}

impl fmt::Debug for SecRun {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SecRun {{ sec: {:?}, dur: {} }}", self.sec, duration_str(&self.dur))
    }
}

/// Runs sections for periods of time
/// 
/// Runs a thread in the background which manages a queue of section runs. Each section runs for a
/// period of time determined by a `Duration`.
pub struct SectionRunner {
    tx: Sender<Op>,
    join_handle: JoinHandle<()>,
}

impl SectionRunner {

    /// Starts a new `SectionRunner` thread and returns a `SectionRunner` struct for managing it
    pub fn start_new() -> SectionRunner {
        let (tx, rx) = channel::<Op>();
        let join_handle = thread::spawn(move|| {
            Self::run(rx)
        });
        SectionRunner { tx: tx, join_handle: join_handle }
    }

    /// Queues a run for section `section` for a period of time specified by `dur`.
    /// 
    /// Returns a `Receiver<[RunNotification](enum.RunNotification.html)>` which can be used to
    /// receive notifications about the progress of the section run. If this `Receiver` is dropped
    /// and closed, it will not cause an error.
    /// 
    /// # Notes
    /// Sections are queued to run in the same order that this method is called in
    pub fn run_section(&self, sec: Arc<Section>, dur: Duration) -> Receiver<RunNotification> {
        let (send, recv) = channel::<RunNotification>();
        let run = SecRun { sec: sec, dur: dur, notification_sender: send };
        self.tx.send(Op::QueueRun(run)).unwrap();
        recv
    }

    /// Tells the `SectionRunner` to stop and waits until the thread quits
    /// 
    /// Any section runs that are queued are discarded, and if a section is currently running it
    /// will be interrupted.
    pub fn stop(self) {
        self.tx.send(Op::Quit).unwrap();
        self.join_handle.join().unwrap();
    }

    /// Runs the thread which does all of the magic
    fn run(rx: Receiver<Op>) {
        use self::Op::*;
        enum Sleep {
            WaitForOp,
            AtMost(Duration),
            None,
        }
        struct Run {
            sec: Arc<Section>,
            dur: Duration,
            start_time: Instant,
            notify: Sender<RunNotification>
        }
        let mut current_run: Option<Run> = None;
        let mut sleep: Sleep = Sleep::WaitForOp;
        let mut queue: VecDeque<SecRun> = VecDeque::new();
        loop {
            let op = match sleep {
                Sleep::WaitForOp   => Some(rx.recv().unwrap()),
                Sleep::AtMost(dur) => match rx.recv_timeout(dur) {
                    Ok(recv) => Some(recv),
                    Err(RecvTimeoutError::Timeout) => None,
                    e @ Err(_) => {
                        e.unwrap();
                        unreachable!()
                    }
                },
                Sleep::None => None,
            };
            trace!("SectionRunner op {:?}", op);
            if let Some(op) = op {
                match op {
                    Quit => {
                        if let Some(run) = current_run {
                            debug!("interrupting section {:?}, ran for {}", run.sec, duration_str(&run.start_time.elapsed()));
                            run.sec.set_state(false);
                            run.notify.send(RunNotification::Interrupted);
                        }
                        return;
                    },
                    QueueRun(run) => queue.push_back(run)
                }
            }
            current_run = {
                if let Some(run) = current_run {
                    let elapsed = run.start_time.elapsed();
                    if elapsed >= run.dur {
                        trace!("finished running section {:?}, ran for {}", run.sec, duration_str(&elapsed));
                        run.sec.set_state(false);
                        run.notify.send(RunNotification::Finish);
                        sleep = Sleep::None;
                        None
                    } else {
                        let sleep_dur = run.dur - elapsed;
                        sleep = Sleep::AtMost(sleep_dur);
                        Some(run)
                    }
                } else {
                    if let Some(run) = queue.pop_front() {
                        trace!("starting running section {:?} for {}", run.sec, duration_str(&run.dur));
                        run.sec.set_state(true);
                        run.notification_sender.send(RunNotification::Start);
                        sleep = Sleep::AtMost(run.dur);
                        Some(Run {
                            start_time: Instant::now(),
                            sec: run.sec,
                            dur: run.dur,
                            notify: run.notification_sender,
                        })
                    } else {
                        sleep = Sleep::WaitForOp;
                        None
                    }
                }
            };
        }
    }
}
