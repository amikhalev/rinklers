//! ScheduleRunner for executing events based on Schedules

use std::thread::{self, JoinHandle};
use std::sync::{Mutex, Condvar};
use std::{cmp, fmt, mem};
use chrono::{DateTime, Local, Duration as CDuration};
use schedule::Schedule;
use util::{LockCondvarGuard, CondvarGuard, chrono_duration_string};

/// Executes events triggered by a [ScheduleRunner](struct.ScheduleRunner.html)
pub trait Executor: Send {
    /// The data that is stored by the `ScheduleRunner` when the event is scheduled and that will
    /// be passed to the executor when the event is triggered.
    type Data: Send + 'static;

    /// Executed when an event is triggered
    fn execute(&self, data: &Self::Data);

    /// Gets the string representation of a `Self::Data` for `Executor`
    fn data_string(data: &Self::Data) -> String {
        let _ = data; // to avoid warning
        "..".to_string()
    }
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

use std::marker::PhantomData;

struct SchedRun<E: Executor> {
    sched: Schedule,
    data: E::Data,
    next_run: Option<DateTime<Local>>,
}

impl<E: Executor> SchedRun<E> {
    fn new(sched: Schedule, data: E::Data) -> Self {
        let mut run = SchedRun {
            sched: sched,
            data: data,
            next_run: None,
        };
        run.update_next_run();
        run
    }

    fn till_run(&self) -> Option<CDuration> {
        self.next_run.map(|next_run| next_run - Local::now())
    }

    fn update_next_run(&mut self) -> Option<DateTime<Local>> {
        self.next_run = self.sched.next_run();
        if let Some(till_run) = self.till_run() {
            trace!("next run for item \"{}\" will be in {:?}",
                   E::data_string(&self.data),
                   chrono_duration_string(&till_run));
        }
        self.next_run
    }
}

impl<E: Executor> fmt::Debug for SchedRun<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SchedRun")
            .field("sched", &self.sched)
            .field("data", &E::data_string(&self.data))
            .field("next_run", &self.next_run)
            .finish()
    }
}

mod index_map {
    use std::mem;
    use std::ops::{Index, IndexMut};
    use std::iter::{Iterator, IntoIterator, FromIterator};
    use std::fmt;

    #[derive(Clone, PartialEq, PartialOrd, Eq, Ord)]
    enum Entry<E> {
        Element(E),
        Empty(Option<usize>),
    }

    #[derive(Clone, PartialEq, PartialOrd, Eq, Ord)]
    pub struct IndexMap<E> {
        entries: Vec<Entry<E>>,
        next_empty: Option<usize>,
    }

    impl<E> IndexMap<E> {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_capacity(capacity: usize) -> Self {
            IndexMap { entries: Vec::with_capacity(capacity), ..Self::new() }
        }

        pub fn capacity(&self) -> usize {
            self.entries.capacity()
        }

        pub fn reserve(&mut self, additional: usize) {
            self.entries.reserve(additional);
        }

        pub fn insert(&mut self, element: E) -> usize {
            let new_entry = Entry::Element(element);
            match self.next_empty.take() {
                None => {
                    let index = self.entries.len();
                    self.entries.push(new_entry);
                    index
                }
                Some(index) => {
                    let empty = mem::replace(&mut self.entries[index], new_entry);
                    let empty = match empty {
                        Entry::Empty(e) => e,
                        Entry::Element(_) => {
                            panic!("IndexMap next_empty not pointing to Entry::Empty")
                        }
                    };
                    self.next_empty = empty;
                    index
                }
            }
        }

        fn remove_entry(next_empty: &mut Option<usize>,
                        idx: usize,
                        entry: &mut Entry<E>)
                        -> Option<E> {
            match *entry {
                Entry::Element(_) => {
                    let element = mem::replace(entry, Entry::Empty(next_empty.take()));
                    *next_empty = Some(idx);
                    match element {
                        Entry::Element(elem) => Some(elem),
                        _ => unreachable!(),
                    }
                }
                Entry::Empty(_) => None,
            }
        }

        pub fn remove(&mut self, idx: usize) -> Option<E> {
            let ref mut entry = self.entries[idx];
            Self::remove_entry(&mut self.next_empty, idx, entry)
        }

        pub fn retain<F>(&mut self, mut f: F)
            where F: FnMut(&E) -> bool
        {
            for (idx, entry) in self.entries.iter_mut().enumerate() {
                let retain = if let Entry::Element(ref elem) = *entry {
                    f(elem)
                } else {
                    true
                };
                if !retain {
                    Self::remove_entry(&mut self.next_empty, idx, entry);
                }
            }
        }

        pub fn indeces(&self) -> Indeces<E> {
            Indeces::new(self)
        }

        pub fn values(&self) -> Values<E> {
            Values::new(self)
        }

        pub fn values_mut(&mut self) -> ValuesMut<E> {
            ValuesMut::new(self)
        }

        pub fn iter(&self) -> Iter<E> {
            Iter::new(self)
        }

        pub fn iter_mut(&mut self) -> IterMut<E> {
            IterMut::new(self)
        }
    }

    impl<E> Default for IndexMap<E> {
        fn default() -> Self {
            IndexMap {
                entries: Vec::default(),
                next_empty: None,
            }
        }
    }

    impl<E> Index<usize> for IndexMap<E> {
        type Output = E;
        fn index(&self, index: usize) -> &Self::Output {
            match self.entries[index] {
                Entry::Element(ref elem) => elem,
                Entry::Empty(_) => panic!("empty entry in IndexMap at index {}", index),
            }
        }
    }

    impl<E> IndexMut<usize> for IndexMap<E> {
        fn index_mut(&mut self, index: usize) -> &mut Self::Output {
            match self.entries[index] {
                Entry::Element(ref mut elem) => elem,
                Entry::Empty(_) => panic!("empty entry in IndexMap at index {}", index),
            }
        }
    }

    impl<'a, E> IntoIterator for &'a IndexMap<E> {
        type Item = (usize, &'a E);
        type IntoIter = Iter<'a, E>;

        fn into_iter(self) -> Iter<'a, E> {
            self.iter()
        }
    }

    impl<'a, E> IntoIterator for &'a mut IndexMap<E> {
        type Item = (usize, &'a mut E);
        type IntoIter = IterMut<'a, E>;

        fn into_iter(mut self) -> IterMut<'a, E> {
            self.iter_mut()
        }
    }

    impl<E> fmt::Debug for IndexMap<E>
        where E: fmt::Debug
    {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.debug_list()
                .entries(self)
                .finish()
        }
    }

    impl<E> FromIterator<E> for IndexMap<E> {
        fn from_iter<T>(iter: T) -> Self
            where T: IntoIterator<Item = E>
        {
            let iter = iter.into_iter();
            let mut map = IndexMap::with_capacity(iter.size_hint().0);
            for item in iter {
                map.insert(item);
            }
            map
        }
    }

    pub struct Iter<'a, E>
        where E: 'a
    {
        index_map: &'a IndexMap<E>,
        index: usize,
    }

    impl<'a, E> Iter<'a, E> {
        fn new(index_map: &'a IndexMap<E>) -> Self {
            Iter {
                index_map: index_map,
                index: 0,
            }
        }
    }

    impl<'a, E> Iterator for Iter<'a, E> {
        type Item = (usize, &'a E);

        fn next(&mut self) -> Option<Self::Item> {
            while self.index < self.index_map.entries.len() {
                let index = self.index;
                self.index += 1;
                match self.index_map.entries[index] {
                    Entry::Empty(_) => continue,
                    Entry::Element(ref e) => return Some((index, e)),
                };
            }
            None
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, Some(self.index_map.entries.len())) // because every entry could be Empty
        }
    }

    pub struct IterMut<'a, E>
        where E: 'a
    {
        index_map: &'a mut IndexMap<E>,
        index: usize,
    }

    impl<'a, E> IterMut<'a, E> {
        fn new(index_map: &'a mut IndexMap<E>) -> Self {
            IterMut {
                index_map: index_map,
                index: 0,
            }
        }
    }

    impl<'a, E> Iterator for IterMut<'a, E> {
        type Item = (usize, &'a mut E);

        fn next(&mut self) -> Option<Self::Item> {
            while self.index < self.index_map.entries.len() {
                let index = self.index;
                self.index += 1;
                // unsafe to avoid possible borrow checker issue with scoping?
                let mut entry: &mut Entry<E> =
                    unsafe { mem::transmute(self.index_map.entries.index_mut(index)) };
                match *entry {
                    Entry::Empty(_) => continue,
                    Entry::Element(ref mut e) => return Some((index, e)),
                };
            }
            None
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, Some(self.index_map.entries.len())) // because every entry could be Empty
        }
    }

    pub struct Indeces<'a, E>
        where E: 'a
    {
        inner: Iter<'a, E>,
    }

    impl<'a, E> Indeces<'a, E> {
        fn new(index_map: &'a IndexMap<E>) -> Self {
            Indeces { inner: Iter::new(index_map) }
        }
    }

    impl<'a, E> Iterator for Indeces<'a, E> {
        type Item = usize;

        #[inline]
        fn next(&mut self) -> Option<Self::Item> {
            self.inner.next().map(|(i, _)| i)
        }
        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            self.inner.size_hint()
        }
    }

    pub struct Values<'a, E>
        where E: 'a
    {
        inner: Iter<'a, E>,
    }

    impl<'a, E> Values<'a, E> {
        fn new(index_map: &'a IndexMap<E>) -> Self {
            Values { inner: Iter::new(index_map) }
        }
    }

    impl<'a, E> Iterator for Values<'a, E> {
        type Item = &'a E;

        #[inline]
        fn next(&mut self) -> Option<Self::Item> {
            self.inner.next().map(|(_, e)| e)
        }
        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            self.inner.size_hint()
        }
    }

    pub struct ValuesMut<'a, E>
        where E: 'a
    {
        inner: IterMut<'a, E>,
    }

    impl<'a, E> ValuesMut<'a, E> {
        fn new(index_map: &'a mut IndexMap<E>) -> Self {
            ValuesMut { inner: IterMut::new(index_map) }
        }
    }

    impl<'a, E> Iterator for ValuesMut<'a, E> {
        type Item = &'a mut E;

        #[inline]
        fn next(&mut self) -> Option<Self::Item> {
            self.inner.next().map(|(_, e)| e)
        }
        #[inline]
        fn size_hint(&self) -> (usize, Option<usize>) {
            self.inner.size_hint()
        }
    }

    #[cfg(test)]
    mod test {
        use super::*;
        use std::iter::FromIterator;

        #[test]
        fn test_index_map() {
            let mut map: IndexMap<i32> = IndexMap::new();

            assert_eq!(map.insert(100), 0);
            assert_eq!(map.insert(200), 1);
            assert_eq!(map.insert(300), 2);
            assert_eq!(map.insert(400), 3);
            assert_eq!(map.indeces().collect::<Vec<_>>(), vec![0, 1, 2, 3]);

            assert_eq!(map.remove(0), Some(100));
            assert_eq!(map.next_empty, Some(0), "map.next_empty");

            assert_eq!(map.indeces().collect::<Vec<_>>(), vec![1, 2, 3]);
            assert_eq!(map.values().cloned().collect::<Vec<_>>(),
                       vec![200, 300, 400]);

            assert_eq!(map.insert(500), 0);

            assert_eq!(map.indeces().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
            assert_eq!(map.values().cloned().collect::<Vec<_>>(),
                       vec![500, 200, 300, 400]);

            let mut map2 = map.clone();
            for item in map2.values_mut() {
                *item += 10;
            }
            let map3 = IndexMap::from_iter(vec![510, 210, 310, 410]);
            assert_eq!(map3.capacity(), 4);
            assert_eq!(map2, map3);
        }
    }
}

use self::index_map::IndexMap;

#[derive(Debug)]
struct RunnerData<E>
    where E: Executor
{
    quit: bool,
    runs: IndexMap<SchedRun<E>>,
}

impl<E: Executor> RunnerData<E> {
    fn new() -> Self {
        RunnerData {
            quit: false,
            runs: IndexMap::new(),
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

    /// Begins an update to the `RunnerState`. When the `CondvarGuard` is dropped, it will notify
    /// the runner thread of the update.
    fn update<'a>(&'a self) -> CondvarGuard<'a, RunnerData<E>> {
        self.data.lock_condvar(&self.condvar)
    }

    /// Notifies the runner thread of an update
    fn notify_update(&self) {
        self.condvar.notify_one();
    }
}

pub struct ScheduleGuard<'a, E>
    where E: Executor + 'static
{
    runner: &'a ScheduleRunner<E>,
    index: usize,
}

impl<'a, E> ScheduleGuard<'a, E>
    where E: Executor
{
    pub fn stop(self) {
        self.runner.remove(self.index);
    }

    pub fn update_schedule(&self, new_schedule: Schedule) {
        self.runner.update_schedule(self.index, new_schedule);
    }

    pub fn update_data(&self, data: E::Data) {
        self.runner.update_data(self.index, data);
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

    /// Schedules an event to be run using the `Executor` whenever the schedule triggers it.
    pub fn schedule<'a>(&'a self, schedule: Schedule, sched_data: E::Data) -> ScheduleGuard<'a, E> {
        let mut data = self.state.update();
        let idx = data.runs.insert(SchedRun::new(schedule, sched_data));
        ScheduleGuard {
            runner: self,
            index: idx,
        }
    }

    fn remove(&self, index: usize) -> Option<SchedRun<E>> {
        let mut data = self.state.update();
        data.runs.remove(index)
    }

    fn update_schedule(&self, index: usize, new_schedule: Schedule) -> Schedule {
        let mut data = self.state.update();
        let ref mut run = data.runs[index];
        mem::replace(&mut run.sched, new_schedule)
    }

    fn update_data(&self, index: usize, new_data: E::Data) -> E::Data {
        let mut data = self.state.update();
        let ref mut run = data.runs[index];
        mem::replace(&mut run.data, new_data)
    }

    /// Stops the `ScheduleRunner`, waiting for the thread to exit
    pub fn stop(self) {
        let mut data = self.state.update();
        data.quit = true;
    }

    fn run(state: &RunnerState<E>, executor: E) {
        let data = &state.data;
        let condvar = &state.condvar;
        use util::{WaitPeriod, wait_condvar};
        let mut wait_period = WaitPeriod::Wait;
        loop {
            trace!("sleeping for {:?}", wait_period);
            let mut data_lock = wait_condvar(&condvar, &data, &wait_period).unwrap();

            // trace!("ScheduleRunner state {:?}", *data_lock);
            if data_lock.quit {
                debug!("quitting ScheduleRunner");
                return;
            }

            wait_period = WaitPeriod::Wait;
            for (_, run) in &mut data_lock.runs {
                if let Some(left) = run.till_run() {
                    if left <= CDuration::zero() {
                        // if the time left is less than or equal to zero, it is time for the
                        // event to run
                        trace!("running scheduled item \"{}\"", E::data_string(&run.data));
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
        let today = now.weekday();
        let schedule = Schedule::new(vec![now.time() + delay_time,
                                          now.time() + delay_time + delay_time],
                                     vec![today],
                                     DateTimeBound::None,
                                     DateTimeBound::None);

        {
            let mut guards: Vec<_> = Vec::new();
            for _ in 0..10 {
                let (tx, rx) = channel::<()>();
                let guard = schedule_runner.schedule(Schedule::default(), Box::new(|| { }));
                guard.update_data(Box::new(move || {
                    tx.send(()).unwrap();
                }));
                guard.update_schedule(schedule.clone());
                guards.push((rx, guard));
            }
            let rxs: Vec<_> = guards.into_iter().map(|(rx, guard)| {
                rx.recv_timeout(Duration::from_millis(50)).ok().expect("schedule did not run in time");
                guard.stop();
                rx
            }).collect();
            for rx in rxs.iter() {
                rx.recv_timeout(Duration::from_millis(50))
                    .err()
                    .expect("schedule ran again when it should not have");
            }
        }

        schedule_runner.stop();
    }
}
