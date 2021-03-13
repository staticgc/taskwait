//! This library provides facility to wait on multiple spawned async tasks.
//! This is runtime agnostic.
//!
//! ## Example using adding and waiting on tasks
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
//! ## Example using add & auto_work
//! This example uses auto_work() which creates a [`Work`] object. When it goes out of scope done() is 
//! is automatically called.
//!
//! ```no_run
//! use taskwait::TaskGroup;
//! 
//! #[tokio::main]
//! async fn main() {
//!     let tg = TaskGroup::new();
//!     for _ in 0..10 {
//!         tg.add(1);
//!         let work = tg.auto_work();
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
//! ## Example using work
//! This example uses work() which creates a [`Work`] object. This calls [`TaskGroup::add`]. When it goes out of scope done() is 
//! is automatically called.
//!
//! ```no_run
//! use taskwait::TaskGroup;
//! 
//! #[tokio::main]
//! async fn main() {
//!     let tg = TaskGroup::new();
//!     for _ in 0..10 {
//!         let work = tg.work();
//!         tokio::spawn(async move{
//!             let _work = work; // done() will be called when this is dropped
//!             //...
//!         });
//!     }
//!     tg.wait().await;
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

    /// Increases/Decreases the task counter.
    ///
    /// This is used to indicate an intention for an upcoming task. Alternatively, this
    /// can be used to decrement the task counter.
    ///
    /// Call to this function should be matched by call to [`Self::done`].
    /// If the call to done() needs to be done in a RAII manner use [`Self::auto_work`]
    /// 

    pub fn add(&self, n: i64) {
        self.inner.add(n);
    }

    /// Creates a work which increments the task counter
    /// The returned [`Work`] decrements the counter when dropped.
    ///
    /// This is equivalent to calling add() & auto_work():
    /// ```no_run
    /// use taskwait::TaskGroup;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let tg = TaskGroup::new();
    ///     tg.add(1);
    ///     let work = tg.auto_work();
    /// }
    /// ```
    pub fn work(&self) -> Work {
        self.add(1);
        self.auto_work()
    }

    /// Creates a work which does not increment the task counter
    /// The returned [`Work`] decrements the counter when dropped.
    /// The only difference between this & work is that [`work`] 
    /// increments the counter, whereas this does not.
    /// 
    /// [`work`]: Self::work
    pub fn auto_work(&self) -> Work {
        Work {
            inner: self.inner.clone(),
        }
    }

    /// Decrements the task counter.
    pub fn done(&self) {
        self.inner.done();
    }

    /// Returns the [`WaitFuture`] 
    /// When awaited the future returns the internal counter, which can be 0 or -ve.
    /// A value can be -ve if there were more [`TaskGroup::done`] calls than [`TaskGroup::add`].
    /// The caller can use this to maintain an invariant in case of any mis-behaving tasks.
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

    fn add(&self, n: i64) {
        // A relaxed ordering should be sufficient because, the
        // add() is always called on a valid & live object
        self.counter.fetch_add(n as i64, Ordering::Relaxed);
    }


    pub fn done(&self) {
        // fetch_sub returns the value before the subtraction.
        // If this is the last done() then subtraction will make value 0 but will return
        // the previous value i.e. 1
        let prev_val = self.counter.fetch_sub(1, Ordering::Release);

        if prev_val <= 1 {
            //Time to wake up the future
            self.waker.wake();
        }
    }
}

/// Represents a work or task.
///
/// When dropped, it decrements the task counter. See [`TaskGroup::work`] & [`TaskGroup::auto_work`]
pub struct Work {
    inner: Arc<Inner>,
}

impl Drop for Work {
    fn drop(&mut self) {
        self.inner.done()
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

    #[tokio::test]
    async fn basic_test_add_auto_work() {
        let tg = TaskGroup::new();
        let num = Arc::new(Mutex::new(0));
        let count = 10000;

        for _ in 0..count {
            tg.add(1);
            let work = tg.auto_work();

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
    async fn basic_test_work() {
        let tg = TaskGroup::new();
        let num = Arc::new(Mutex::new(0));
        let count = 10000;

        for _ in 0..count {
            let work = tg.work();
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
    async fn neg_wait() {
        // We wait after the internal counter becomes negative

        let tg = TaskGroup::new();

        // Decrement internal counter to negative
        tg.done();

        let n = tg.wait().await;
        assert_eq!(-1, n);
    }
}
