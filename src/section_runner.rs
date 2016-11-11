use std;
use std::time::{Duration, Instant};
use std::sync::mpsc::{Sender, Receiver, channel, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::collections::VecDeque;
use std::fmt;
use section::Section;

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

#[derive(Debug)]
enum Op {
    Quit,
    QueueRun(SecRun)
}

pub struct SectionRunner {
    tx: Sender<Op>,
    join_handle: Option<JoinHandle<()>>,
}

struct SecRun {
    sec: &'static Section,
    dur: Duration
}

impl SecRun {
    unsafe fn new_transmute<'a>(sec: &'a Section, dur: Duration) -> SecRun {
        SecRun { sec: std::intrinsics::transmute(sec), dur: dur }
    }
}

impl fmt::Debug for SecRun {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SecRun {{ sec: {:?}, dur: {} }}", self.sec, duration_str(&self.dur))
    }
}

impl SectionRunner {
    pub fn start_new() -> SectionRunner {
        let (tx, rx) = channel::<Op>();
        let join_handle = thread::spawn(move|| {
            Self::run(rx)
        });
        SectionRunner { tx: tx, join_handle: Some(join_handle) }
    }

    pub fn queue_section_run<'a>(&'a self, sec: &'a Section, dur: Duration) {
        let run = unsafe { SecRun::new_transmute(sec, dur) };
        self.tx.send(Op::QueueRun(run)).unwrap()
    }

    // must be called to stop the thread, or else it will join and never finish
    pub fn stop(self) {
        self.tx.send(Op::Quit).unwrap();
        // drops self, which joins the thread
    }

    fn run(rx: Receiver<Op>) {
        use self::Op::*;
        enum Sleep {
            WaitForOp,
            AtMost(Duration),
            None,
        }
        struct Run {
            sec: &'static Section,
            dur: Duration,
            start_time: Instant,
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
                        sleep = Sleep::AtMost(run.dur);
                        Some(Run {
                            start_time: Instant::now(),
                            sec: run.sec,
                            dur: run.dur,
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

impl Drop for SectionRunner {
    fn drop(&mut self) {
        // since this joins the thread, the transmute with SecRun should be ok...
        // any sections sent to SectionRunner have to live longer than the runner,
        // and as long as the runner is dropped the thread should be stopped. should...
        if let Some(join_handle) = std::mem::replace(&mut self.join_handle, None) {
            join_handle.join().unwrap();
        }
    }
}
