use std::fmt;
use std::sync::Mutex;
use std::cell::Cell;

pub trait Section: Send + Sync {
    fn name(&self) -> &String;
    fn set_state(&self, state: bool);
    fn state(&self) -> bool;
}

impl fmt::Debug for Section {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Section {{ name: \"{}\", state: {} }}", self.name(), self.state())
    }
}

pub struct LogSection {
    name: String,
    state: Mutex<Cell<bool>>,
}

impl LogSection {
    pub fn new<S: Into<String>>(name: S) -> LogSection {
        LogSection { name: name.into(), state: Mutex::new(Cell::new(false)) }
    }
}

impl Section for LogSection {
    fn name(&self) -> &String {
        &self.name
    }

    fn set_state(&self, state: bool) {
        debug!("setting section {} state to {}", self.name, state);
        self.state.lock().unwrap().set(state);
    }

    fn state(&self) -> bool {
        self.state.lock().unwrap().get()
    }
}

