//! Crate which provides sync primitives with as main goal
//! to allow an async application making use of the Tokio runtime
//! to be able to gracefully shutdown, meaning that ideally the server process exits
//! only when all active tasks were done with their ongoing work.
//!
//! ## Example
//!
//! ```rust
//! use std::time::Duration;
//! use tokio_task_manager::TaskManager;
//!
//! #[tokio::main]
//! async fn main() {
//!     // An application requires only a single TaskManager,
//!     // created usually where you also build and start the Tokio runtime.
//!     let tm = TaskManager::new(Duration::from_millis(200));
//!
//!     // In this example we spawn 10 tasks,
//!     // where each of them also spawns a task themselves,
//!     // resulting in a total of 20 tasks which we'll want to wait for.
//!     let (tx, mut rx) = tokio::sync::mpsc::channel(20);
//!     for i in 0..10 {
//!         let tx = tx.clone();
//!         let n = i;
//!
//!         // create a task per task that we spawn, such that:
//!         // - the application can wait until the task is dropped,
//!         //   identifying the spawned task is finished;
//!         // - the spawn task knows that the application is gracefully shutting down (.wait);
//!         let mut task = tm.task();
//!         tokio::spawn(async move {
//!             // spawn also child task to test task cloning,
//!             // a task is typically cloned for tasks within tasks,
//!             // each cloned task also needs to be dropped prior to
//!             // the application being able to gracefully shut down.
//!             let mut child_task = task.clone();
//!             let child_tx = tx.clone();
//!             let m = n;
//!             tokio::spawn(async move {
//!                 tokio::time::sleep(Duration::from_millis(m * 10)).await;
//!                 // Using the tokio::select! macro you can allow a task
//!                 // to either get to its desired work, or quit already
//!                 // in case the application is planning to shut down.
//!                 //
//!                 // A typical use case of this is a server which is waiting
//!                 // for an incoming request, which is a text-book example
//!                 // of a task in idle state.
//!                 tokio::select! {
//!                     result = child_tx.send((m+1)*10) => assert!(result.is_ok()),
//!                     _ = child_task.wait() => (),
//!                 }
//!             });
//!             // Do the actual work.
//!             tokio::time::sleep(Duration::from_millis(n * 10)).await;
//!             tokio::select! {
//!                 result = tx.send(n) => assert!(result.is_ok()),
//!                 _ = task.wait() => (),
//!             }
//!         });
//!     }
//!
//!     // we also create a task for something that will never finish,
//!     // just to show that the tokio::select! approach does work...
//!     let mut task = tm.task();
//!     tokio::spawn(async move {
//!         // spawn also child task to test task cloning
//!         let mut child_task = task.clone();
//!         tokio::spawn(async move {
//!             // should shut down rather than block for too long
//!             tokio::select! {
//!                 _ = child_task.wait() => (),
//!                 _ = tokio::time::sleep(Duration::from_secs(60)) => (),
//!             }
//!         });
//!         // should shut down rather than block for too long
//!         tokio::select! {
//!             _ = task.wait() => (),
//!             _ = tokio::time::sleep(Duration::from_secs(60)) => (),
//!         }
//!     });
//!
//!     // sleep for 100ms, just to ensure that all child tasks have been spawned as well
//!     tokio::time::sleep(Duration::from_millis(100)).await;
//!
//!     // drop our sender such that rx.recv().await will return None,
//!     // once our other senders have been dropped and the channel's buffer is empty
//!     drop(tx);
//!
//!     // notify all spawned tasks that we wish to gracefully shut down
//!     // and wait until they do. The resulting boolean is true if the
//!     // waiting terminated gracefully (meaning all tasks quit on their own while they were idle).
//!     assert!(tm.wait().await);
//!
//!     // collect all our results,
//!     // which we can do all at once given the channel
//!     // was created with a sufficient buffer size.
//!     let mut results = Vec::with_capacity(20);
//!     while let Some(n) = rx.recv().await {
//!         results.push(n);
//!     }
//!
//!     // test to proof we received all expected results
//!     results.sort_unstable();
//!     assert_eq!(
//!         &results,
//!         &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
//!     );
//! }
//! ```
//!
//! In case your application's root tasks are infinite loops you might wish
//! to gracefully shutdown only once a SIGINT (CTRL+C) signal has been received.
//! In this case you would instead of `tm.wait().await` do instead:
//! `tm.shutdown_gracefully_on_ctrl_c().await`.

use std::time::Duration;
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing::debug;

/// A task allows a shutdown to happen gracefully by waiting
/// until the spawned Tokio task is finished and it also allows
/// the Tokio task to know when a shutdown is requested,
/// to be listened to on an idle moment (e.g. while waiting for an incoming network request).
pub struct Task {
    tx: mpsc::Sender<()>,
    btx: broadcast::Sender<()>,
    rx: broadcast::Receiver<()>,
}

impl Task {
    /// Wait for a global shutdown signal to be received,
    /// when it is received it is expected that the Tokio task exits immediately.
    pub async fn wait(&mut self) {
        _ = self.rx.recv().await;
        debug!("task received shutdown signal");
    }
}

impl Clone for Task {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            btx: self.btx.clone(),
            rx: self.btx.subscribe(),
        }
    }
}

/// The task manager is used similar to how in Go a WaitGroup is used.
/// It provides us with the ability to gracefully shutdown the application
/// giving the chance for each spawned task to gracefully exit during an idle moment.
///
/// Normally the graceful shutdown will be induced by a system signal (e.g. SIGINT),
/// but spawned tasks can also induce it themselves if required for critical reasons.
pub struct TaskManager {
    wait_timeout: Duration,

    btx: broadcast::Sender<()>,
    rtx: broadcast::Receiver<()>,
    tx: mpsc::Sender<()>,
    rx: mpsc::Receiver<()>,
}

impl TaskManager {
    /// Create a new task manager.
    /// There should be only one manager per application.
    pub fn new(wait_timeout: Duration) -> Self {
        let (btx, rtx) = broadcast::channel(1);
        let (tx, rx) = mpsc::channel(1);

        Self {
            wait_timeout,

            btx,
            rtx,
            tx,
            rx,
        }
    }

    // Spawn a task, to be used for any Tokio task spawned,
    // which will ensure that any task spawned gets the chance to gracefully shutdown.
    pub fn task(&self) -> Task {
        Task {
            tx: self.tx.clone(),
            btx: self.btx.clone(),
            rx: self.btx.subscribe(),
        }
    }

    /// Wait for all tasks to finish,
    /// or until the defined timeout has been reached.
    ///
    /// Returns a boolean indicating if the shutdown was graceful.
    pub async fn wait(self) -> bool {
        self.shutdown(false).await
    }

    /// Block the shutdown process until a CTRL+C signal has been received,
    /// and shutdown gracefully once received.
    ///
    /// In case no tasks are active we will immediately return as well,
    /// preventing programs from halting in case infinite loop tasks
    /// exited early due to an error.
    ///
    /// Returns a boolean indicating if the shutdown was graceful,
    /// or in case the process shut down early (no tasks = graceful by definition).
    pub async fn shutdown_gracefully_on_ctrl_c(self) -> bool {
        self.shutdown(true).await
    }

    async fn shutdown(mut self, block_until_signal: bool) -> bool {
        // drop our own sender, such that we can check if our rx.recv exits with an error,
        // which would mean that all active tasks have quit
        drop(self.tx);

        // only if the user wishes to block until a signal is received will we do so,
        // for some purposes this however not desired and instead a downstream
        // signal from manager to tasks is desired as a trigger instead
        if block_until_signal {
            tokio::select! {
                _ = self.rx.recv() => {
                    debug!("task manager has been shut down due to no active tasks");
                    // exit early, as no signalling and waiting has to be done when no tasks are active
                    return true;
                },
                _ = signal::ctrl_c() => {
                    debug!("ctrl+c signal received: starting graceful shutdown of task manager");
                }
            };
        }

        // signal that all tasks have to stop once they can
        if let Err(err) = self.btx.send(()) {
            debug!(
                "task manager received error while sending broadcast shutdown signal: {}",
                err
            );
        }
        // drop our own receiver
        drop(self.rtx);

        if let Err(err) = timeout(self.wait_timeout, async move { _ = self.rx.recv().await }).await
        {
            debug!("task manager received error while waiting: {}", err);
            return false;
        }

        true
    }
}

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDocTests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_manager_zero_task_wait() {
        let tm = TaskManager::new(Duration::from_secs(1));
        assert!(tm.wait().await);
    }

    #[tokio::test]
    async fn task_manager_zero_task_shutdown_gracefully_on_ctrl_c_() {
        let tm = TaskManager::new(Duration::from_secs(1));
        assert!(tm.shutdown_gracefully_on_ctrl_c().await);
    }

    #[tokio::test]
    async fn task_manager_graceful_shutdown() {
        let tm = TaskManager::new(Duration::from_millis(200));
        let (tx, mut rx) = tokio::sync::mpsc::channel(20);
        for i in 0..10 {
            let tx = tx.clone();
            let n = i;
            let mut task = tm.task();
            tokio::spawn(async move {
                // spawn also child task to test task cloning
                let mut child_task = task.clone();
                let child_tx = tx.clone();
                let m = n;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(m * 10)).await;
                    tokio::select! {
                        result = child_tx.send((m+1)*10) => assert!(result.is_ok()),
                        _ = child_task.wait() => (),
                    }
                });
                // do the actual work
                tokio::time::sleep(Duration::from_millis(n * 10)).await;
                tokio::select! {
                    result = tx.send(n) => assert!(result.is_ok()),
                    _ = task.wait() => (),
                }
            });
        }
        let mut task = tm.task();
        tokio::spawn(async move {
            // spawn also child task to test task cloning
            let mut child_task = task.clone();
            tokio::spawn(async move {
                // should shut down rather than block for too long
                tokio::select! {
                    _ = child_task.wait() => (),
                    _ = tokio::time::sleep(Duration::from_secs(60)) => (),
                }
            });
            // should shut down rather than block for too long
            tokio::select! {
                _ = task.wait() => (),
                _ = tokio::time::sleep(Duration::from_secs(60)) => (),
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(tx);
        assert!(tm.wait().await);
        let mut results = Vec::with_capacity(20);
        while let Some(n) = rx.recv().await {
            results.push(n);
        }
        results.sort_unstable();
        assert_eq!(
            &results,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
        );
    }

    #[tokio::test]
    async fn task_manager_shutdown_timeout() {
        let tm = TaskManager::new(Duration::from_millis(10));

        let mut task = tm.task();
        tokio::spawn(async move {
            let _ = task.wait();
            // thread takes too long, should force to shut down
            tokio::time::sleep(Duration::from_secs(60)).await;
            panic!("never should reach here");
        });

        assert!(!tm.wait().await);
    }
}
