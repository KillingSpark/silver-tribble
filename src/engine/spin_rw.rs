use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

pub struct SpinRwLock<T> {
    data: UnsafeCell<T>,
    readers: AtomicUsize,
    writer_waiting: AtomicUsize,
}

unsafe impl<T> Sync for SpinRwLock<T> {}
unsafe impl<T> Send for SpinRwLock<T> {}

const WRITER_ACTIVE: usize = usize::MAX;

impl<T> SpinRwLock<T> {
    pub fn new(t: T) -> Self {
        Self {
            data: UnsafeCell::new(t),
            readers: AtomicUsize::new(0),
            writer_waiting: AtomicUsize::new(0),
        }
    }

    pub fn read<'a>(&'a self) -> SpinReadLockGuard<'a, T> {
        loop {
            // keep looping if there is a writer waiting
            // THIS WILL STARVE READERS. That is explicitly wanted for this specific use case though
            if self.writer_waiting.load(Ordering::Acquire) > 0 {
                continue;
            }

            let readers = self.readers.load(Ordering::Acquire);

            // keep looping while writer is active
            if readers == WRITER_ACTIVE {
                continue;
            }

            let new_readers = readers + 1;

            if self
                .readers
                .compare_exchange_weak(readers, new_readers, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        SpinReadLockGuard { lock: self }
    }


    #[allow(dead_code)]
    pub fn try_read<'a>(&'a self) -> Option<SpinReadLockGuard<'a, T>> {
        if self.writer_waiting.load(Ordering::Acquire) > 0 {
            return None;
        }

        let readers = self.readers.load(Ordering::Acquire);
        let new_readers = readers + 1;

        // keep looping while writer is active
        if new_readers >= WRITER_ACTIVE {
            return None;
        }

        if self
            .readers
            .compare_exchange(readers, new_readers, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Some(SpinReadLockGuard { lock: self })
        } else {
            None
        }
    }

    pub fn write<'a>(&'a self) -> SpinWriteLockGuard<'a, T> {
        self.writer_waiting.fetch_add(1, Ordering::AcqRel);
        loop {
            if self
                .readers
                .compare_exchange_weak(0, WRITER_ACTIVE, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        self.writer_waiting.fetch_sub(1, Ordering::AcqRel);
        SpinWriteLockGuard { lock: self }
    }

    #[allow(dead_code)]
    pub fn try_write<'a>(&'a self) -> Option<SpinWriteLockGuard<'a, T>> {
        // let's give the writers that do a blocking operation the advantage
        if self.writer_waiting.load(Ordering::Acquire) > 0 {
            return None;
        }

        if self
            .readers
            .compare_exchange(0, WRITER_ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Some(SpinWriteLockGuard { lock: self })
        } else {
            None
        }
    }
}

pub struct SpinReadLockGuard<'a, T> {
    lock: &'a SpinRwLock<T>,
}

impl<'a, T> SpinReadLockGuard<'a, T> {
    pub fn get(&'a self) -> &'a T {
        unsafe { self.lock.data.get().as_mut().unwrap() }
    }
}

impl<'a, T> Drop for SpinReadLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.readers.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct SpinWriteLockGuard<'a, T> {
    lock: &'a SpinRwLock<T>,
}

impl<'a, T> SpinWriteLockGuard<'a, T> {
    pub fn get(&'a self) -> &'a mut T {
        unsafe { self.lock.data.get().as_mut().unwrap() }
    }
}

impl<'a, T> Drop for SpinWriteLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.readers.store(0, Ordering::Release);
    }
}
