//! Implementation of irrigation zones (or [Section](trait.Section.html)s)
//!
//! All section types implement the [Section](trait.Section.html) trait. They generally map to some
//! real-world control of section state (ie. a GPIO pin).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

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
        write!(f,
               "Section {{ name: \"{}\", state: {} }}",
               self.name(),
               self.state())
    }
}

/// A [Section](trait.Section.html) which holds a name and state, but performs no real world
/// actions
pub struct NoopSection {
    name: String,
    state: AtomicBool,
}

impl NoopSection {
    /// Creates a new `NoopSection` with the specified `name` and `false` for the state.
    pub fn new<S: Into<String>>(name: S) -> Self {
        NoopSection {
            name: name.into(),
            state: AtomicBool::new(false),
        }
    }
}

impl Section for NoopSection {
    #[inline]
    fn name(&self) -> &String {
        &self.name
    }

    #[inline]
    fn set_state(&self, state: bool) {
        self.state.store(state, Ordering::Relaxed);
    }

    #[inline]
    fn state(&self) -> bool {
        self.state.load(Ordering::Relaxed)
    }
}

/// A [Section](trait.Section.html) implementation which wraps another section and adds logging for
/// whenever the state is set
pub struct LogSection<S: Section> {
    section: S,
}

impl<S: Section> LogSection<S> {
    /// Creates a new `LogSection` wrapping the specified `section`
    pub fn new(section: S) -> Self {
        LogSection { section: section }
    }
}

impl LogSection<NoopSection> {
    /// Creates a new `LogSection` wrapping a `NoopSection`, essentially a section only logging and
    /// storing state.
    pub fn new_noop(name: String) -> Self {
        Self::new(NoopSection::new(name))
    }
}

impl<S: Section> Section for LogSection<S> {
    #[inline]
    fn name(&self) -> &String {
        self.section.name()
    }

    #[inline]
    fn set_state(&self, state: bool) {
        debug!("setting section {} state to {}", self.name(), state);
        self.section.set_state(state);
    }

    #[inline]
    fn state(&self) -> bool {
        self.section.state()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    pub struct TestSection<S, F>
        where S: Section,
              F: Fn(bool) + Send + Sync + 'static
    {
        section: S,
        fun: F,
    }

    impl<S, F> TestSection<S, F>
        where S: Section,
              F: Fn(bool) + Send + Sync + 'static
    {
        pub fn new(section: S, fun: F) -> Self {
            TestSection {
                section: section,
                fun: fun,
            }
        }
    }

    impl<S, F> Section for TestSection<S, F>
        where S: Section,
              F: Fn(bool) + Send + Sync + 'static
    {
        #[inline]
        fn name(&self) -> &String {
            &self.section.name()
        }

        #[inline]
        fn set_state(&self, state: bool) {
            (self.fun)(state);
            self.section.set_state(state);
        }

        #[inline]
        fn state(&self) -> bool {
            self.section.state()
        }
    }
}
