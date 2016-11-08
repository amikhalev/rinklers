#[macro_use]
extern crate log;
extern crate env_logger;

use std::collections::VecDeque;
use std::time::Duration;
use std::thread;
use std::cell::Cell;

trait Section {
    fn name(&self) -> &String;
    fn set_state(&self, state: bool);
    fn state(&self) -> bool;
}

struct LogSection {
    name: String,
    state: Cell<bool>,
}

impl LogSection {
    fn new<S: Into<String>>(name: S) -> LogSection {
        LogSection { name: name.into(), state: Cell::new(false) }
    }
}

impl Section for LogSection {
    fn name(&self) -> &String {
        &self.name
    }

    fn set_state(&self, state: bool) {
        debug!("setting section {} state to {}", self.name, state);
        self.state.set(state);
    }

    fn state(&self) -> bool {
        self.state.get()
    }
}

struct SectionRunner<'a> {
    queue: VecDeque<SectionRun<'a>>
}

struct SectionRun<'a> {
    sec: &'a Section,
    duration: Duration
}

impl<'a> SectionRunner<'a> {
    pub fn new() -> SectionRunner<'a> {
        SectionRunner { queue: VecDeque::new() }
    }

    fn queue_section_run(&mut self, sec: &'a Section, duration: Duration) {
        self.queue.push_back(SectionRun {sec: sec, duration: duration});
    }

    fn run(&mut self) {
        while let Some(run) = self.queue.pop_front() {
            info!("running section {} for {}", run.sec.name(), duration_str(&run.duration));
            run.sec.set_state(true);
            thread::sleep(run.duration);
            run.sec.set_state(false);
        }
    }
}

fn main() {
    env_logger::init().unwrap();

    info!("initializing sections");
    let mut sections: Vec<Box<Section>>;
    sections = (1..6+1)
        .map(|i| format!("Section {}", i))
        .map(|name| Box::new(LogSection::new(name)) as Box<Section>)
        .collect();
    let mut section_runner = SectionRunner::new();
    for section in sections.iter_mut() {
        section.set_state(false);
        section_runner.queue_section_run(&**section, Duration::new(123456, 12346563));
    }

    section_runner.run();
}

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

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
