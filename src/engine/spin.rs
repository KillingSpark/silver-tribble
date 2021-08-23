use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct SpinLock<T> {
    data: UnsafeCell<T>,
    locker: AtomicBool,
}
const LOCKED: bool = false;
const UNLOCKED: bool = true;

unsafe impl<T> Sync for SpinLock<T> {}
unsafe impl<T> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub fn new(t: T) -> Self {
        Self {
            data: UnsafeCell::new(t),
            locker: AtomicBool::new(UNLOCKED),
        }
    }

    #[allow(dead_code)]
    pub fn lock<'a>(&'a self) -> SpinLockGuard<'a, T> {
        loop {
            if self
                .locker
                .compare_exchange_weak(UNLOCKED, LOCKED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        SpinLockGuard { lock: self }
    }

    pub fn try_lock<'a>(&'a self) -> Option<SpinLockGuard<'a, T>> {
        if self
            .locker
            .compare_exchange(UNLOCKED, LOCKED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Some(SpinLockGuard { lock: self })
        } else {
            None
        }
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> SpinLockGuard<'a, T> {
    pub fn get(&'a self) -> &'a mut T {
        unsafe { self.lock.data.get().as_mut().unwrap() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock
            .locker
            .store(UNLOCKED, Ordering::Release);
    }
}
