//! Implementation of irrigation zones (or [Section](trait.Section.html)s)
//! 
//! All section types implement the [Section](trait.Section.html) trait. They generally map to some
//! real-world control of section state (ie. a GPIO pin).

use std::fmt;
use std::sync::Mutex;
use std::cell::Cell;

/// Represents a single zone in the sprinkler system. Structs that implement this trait must be
/// thread-safe, as is indicated by the `Send + Sync` trait bound.
pub trait Section: Send + Sync {
    /// Gets the name of the section, which should be an immutable human readable string
    fn name(&self) -> &String;

    /// Sets the current state of the section. `true` means that it is currently irrigating.
    fn set_state(&self, state: bool);

    /// Gets the current state of the section. Corresponds with [set_state](#tymethod.set_state)
    fn state(&self) -> bool;
}

impl fmt::Debug for Section {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Section {{ name: \"{}\", state: {} }}", self.name(), self.state())
    }
}

/// A [Section](trait.Section.html) implementation which does not perform any actions besides
/// logging.
pub struct LogSection {
    name: String,
    state: Mutex<Cell<bool>>,
}

impl LogSection {
    /// Creates a new `LogSection` with the specified `name`
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

