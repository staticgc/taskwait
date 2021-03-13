//! This library provides facility to wait on multiple spawned async tasks.
//! It is runtime agnostic.
//!
//! ## Adding and waiting on tasks
//! ```no_run
//! use taskwait::TaskGroup;
//! 
//! #[tokio::main]
//! async fn main() {
//!     let tg = TaskGroup::new();
//!     for _ in 0..10 {
//!         tg.add(1);
//!         let tg_c = tg.clone();
//!         tokio::spawn(async move{
//!             //...
//!             tg_c.done();
//!         });
//!     }
//!     tg.wait().await;
//! }
//! ```
//! 
//! Note: User must ensure that call to done() made above is made in both success & error code path.
//!
//! ## Using add_work
//! This example uses add_work() which creates a [`Work`] object. When it goes out of scope done() is 
//! is automatically called.
//!
//! ```no_run
//! use taskwait::TaskGroup;
//! 
//! #[tokio::main]
//! async fn main() {
//!     let tg = TaskGroup::new();
//!     for _ in 0..10 {
//!         let work = tg.add_work(1);
//!         tokio::spawn(async move{
//!             let _work = work; // done() will be called when this is dropped
//!             //...
//!         });
//!     }
//! 
//!     tg.wait().await;
//! }
//! ```
//! 
//! ## Reusing the taskgroup
//! The following shows how the same task group can be reused to achieve checkpointing
//!
//! ```no_run
//! use taskwait::{TaskGroup, Work};
//!
//! async do_some_tasks(tg: TaskGroup, count: usize) {
//!     for _ in 0..count {
//!         let work = tg.add_work(1);
//!         tokio::spawn(async move {
//!             let _work = work
//!             // .. do something
//!         });
//!     }
//! }
//! 
//! #[tokio::main]
//! async fn main() {
//!     let tg = TaskGroup::new();
//!     tokio::spawn(tg.clone(), 100); // Spawn 100 tasks
//!     wg.wait().await; // Let the first 100 complete first. 
//!     
//!     tokio::spawn(tg.clone(), 100); // Spawn 100 tasks
//!     tg.wait().await; // Wait for the next 100
//! }
//! ```

use std::sync::{Arc, atomic::{AtomicI64, Ordering}};
use futures_util::{{future::Future}, task::{Context, AtomicWaker, Poll}};
use std::pin::Pin;

#[derive(Clone)]
pub struct TaskGroup {
    inner: Arc<Inner>
}

impl TaskGroup {
    /// Creates new task group
    pub fn new() -> Self {
        TaskGroup {
            inner: Arc::new(Inner::new()),
        }
    }

    /// Increases the task counter.
    ///
    /// This is used to indicate an intention for an upcoming task. Alternatively, this
    /// can be used to decrement the task counter.
    ///
    /// Call to this function should be matched by call to [`Self::done`].
    /// If the call to done() needs to be done in a RAII manner use [`Self::add_work`]
    pub fn add(&self, n: u32) {
        self.inner.add(n);
    }

    /// Creates a work which does increment the task counter by `n`.
    /// The returned [`Work`] decrements the counter when dropped.
    pub fn add_work(&self, n: u32) -> Work {
        self.add(n);

        Work {
            n,
            inner: self.inner.clone(),
        }
    }

    /// Decrements the task counter.
    pub fn done(&self) {
        self.inner.done();
    }

    /// Decrements the task counter by `n`
    pub fn done_n(&self, n: u32) {
        self.inner.done_n(n);
    }

    /// Returns the [`WaitFuture`] 
    /// When awaited the future returns the internal counter, which can be 0 or -ve.
    /// A value can be -ve if there were more [`TaskGroup::done`] calls than [`TaskGroup::add`].
    /// The caller can use this to maintain an invariant in case of any mis-behaving tasks.
    ///
    /// The future when resolved also resets the internal counter to zero. The taskgroup then can 
    /// be reused.
    ///
    /// ```no_run
    /// use taskwait::TaskGroup;
    /// 
    /// #[tokio::main]
    /// async fn main() {
    ///     let tg = TaskGroup::new();
    ///     
    ///     // ... Spawn tasks ...
    ///
    ///     let n = tg.wait().await;
    ///     if n < 0 {
    ///         // Return Error
    ///     }
    /// }
    /// ```
    pub fn wait(&self) -> WaitFuture {
        WaitFuture {
            inner: self.inner.clone(),
        }
    }
}

struct Inner {
    // Counter to keep track of outstanding work
    counter: AtomicI64,

    //Waker to wake the future
    waker: AtomicWaker,
}

impl Inner {
    fn new() -> Self {
        Inner {
            counter: AtomicI64::new(0),
            waker: AtomicWaker::new(),
        }
    }

    fn reset(&self) {
        self.counter.store(0, Ordering::Relaxed);
    }

    fn add(&self, n: u32) {
        if n == 0 {
            return
        }
        // A relaxed ordering should be sufficient because, the
        // add() is always called on a valid & live object
        self.counter.fetch_add(n as i64, Ordering::Relaxed);
    }

    fn done_n(&self, n: u32) {
        if n == 0 {
            return
        }

        let n = n as i64;
        // fetch_sub returns the value before the subtraction.
        // If this is the last done() then subtraction will make value 0 but will return
        // the previous value 
        let prev_val = self.counter.fetch_sub(n, Ordering::Release);

        if prev_val - n   <= 1 {
            //Time to wake up the future
            self.waker.wake();
        }
    }

    pub fn done(&self) {
        self.done_n(1);
    }
}

/// Represents a work or task.
///
/// When dropped, it decrements the task counter. See [`TaskGroup::add_work`]
pub struct Work {
    n: u32,
    inner: Arc<Inner>,
}

impl Drop for Work {
    fn drop(&mut self) {
        self.inner.done_n(self.n)
    }
}

/// Future to wait for the counter to become 0 or -ve.
///
/// See [`TaskGroup::wait`]
pub struct WaitFuture {
    inner: Arc<Inner>,
}

impl Future for WaitFuture {
    type Output = i64;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let n = self.inner.counter.load(Ordering::Acquire);
        if n <= 0 {
            self.inner.reset();
            Poll::Ready(n)
        }else{
            self.inner.waker.register(cx.waker());
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;
    use tokio::sync::Mutex;
    use super::*;

    #[tokio::test]
    async fn basic_test_add() {
        let tg = TaskGroup::new();
        let num = Arc::new(Mutex::new(0));
        let count = 10000;

        for _ in 0..count {
            tg.add(1);
            
            let tg_c = tg.clone();
            let n = num.clone();
            tokio::spawn(async move {
                {
                    let mut n = n.lock().await;
                    *n += 1;
                }

                tg_c.done();
            });
        }

        tg.wait().await;

        let n = num.lock().await;
        assert_eq!(count, *n);
    }

    async fn _basic_test_add_work(tg: TaskGroup) {
        let num = Arc::new(Mutex::new(0));
        let count = 10000;
        for _ in 0..count {
            let work = tg.add_work(1);

            let n = num.clone();
            tokio::spawn(async move {
                let _work = work;
                let mut n = n.lock().await;
                *n += 1;
            });
        }

        tg.wait().await;

        let n = num.lock().await;
        assert_eq!(count, *n);
    }

    #[tokio::test]
    async fn basic_test_add_work() {
        let tg = TaskGroup::new();
        _basic_test_add_work(tg).await;
    }

    #[tokio::test]
    async fn basic_test_add_work_resuse() {
        let tg = TaskGroup::new();
        _basic_test_add_work(tg.clone()).await;
        _basic_test_add_work(tg).await;
    }

    #[tokio::test]
    async fn basic_test_addn_workn() {
        let tg = TaskGroup::new();
        let num = Arc::new(Mutex::new(0));
        let count = 10000;

        let work = tg.add_work(count);
        let num_c = num.clone();

        tokio::spawn(async move {
            let _work = work;
            let mut hvec = Vec::new();

            for _ in 0..count {
                let n = num_c.clone();
                let h = tokio::spawn(async move {
                    let mut n = n.lock().await;
                    *n += 1;
                });
                hvec.push(h);
            }

            for h in hvec {
                let _ = h.await;
            }

        });

        tg.wait().await;

        let n = num.lock().await;
        assert_eq!(count, *n);
    }

    #[tokio::test]
    async fn basic_test_addn_donen() {
        let tg = TaskGroup::new();
        let num = Arc::new(Mutex::new(0));
        let count = 10000;

        tg.add(count);
        let num_c = num.clone();

        let tg_c = tg.clone();

        tokio::spawn(async move {
            let mut hvec = Vec::new();

            for _ in 0..count {
                let n = num_c.clone();
                let h = tokio::spawn(async move {
                    let mut n = n.lock().await;
                    *n += 1;
                });
                hvec.push(h);
            }

            for h in hvec {
                let _ = h.await;
            }

            tg_c.done_n(count); 
        });

        tg.wait().await;

        let n = num.lock().await;
        assert_eq!(count, *n);
    }

    #[tokio::test]
    async fn basic_test_add0_work() {
        let tg = TaskGroup::new();

        let count = 10000;
        let _work_0 = tg.add_work(0);
        let work = tg.add_work(count);
        drop(work);

        tg.wait().await;
    }

    #[tokio::test]
    async fn neg_wait() {
        let tg = TaskGroup::new();
        let count = 1000;
        let _work = tg.add_work(count);

        // Decrement internal counter to negative
        tg.done_n(count + 1);

        let n = tg.wait().await;
        assert_eq!(-1, n);
    }
}
