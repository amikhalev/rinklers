use std::sync::Mutex;
use std::collections::VecDeque;

pub struct BlockingQueue<T> {
    queue: Mutex<VecDeque<T>>,
    condvar: Condvar,
}

impl<T> BlockingQueue<T> {
    pub fn with_capacity(cap: usize) -> BlockingQueue<T> {
        BlockingQueue {
            queue: Mutex::new(VecDeque::with_capacity(cap)),
            condvar: Condvar::new(),
        }
    }

    pub fn push_back(&mut self, item: T) {
        let mut queue = self.queue.lock().unwrap();
        queue.push_back(item);
        self.condvar.notify_one();
    }

    pub fn wait(&self) -> MutexGuard<&VecDeque<T>> {
        let queue = self.queue.lock().unwrap();
        self.condvar.wait(queue).unwrap()
    }
}
