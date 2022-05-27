//! Crate which provides sync primitives with as main goal
//! to allow an async application making use of the Tokio runtime
//! to be able to gracefully shutdown, meaning that ideally the server process exits
//! only when all active tasks were done with their ongoing work.

use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing::{debug, error};

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
    pub async fn wait(mut self) -> bool {
        // signal that all tasks have to stop once they can
        if let Err(err) = self.btx.send(()) {
            error!(
                "task manager received error while sending broadcast shutdown signal: {}",
                err
            );
        }
        // drop our own receiver
        drop(self.rtx);

        // drop our own sender,
        // and wait until all other senders are dropped as well (at which point rx.recv() will exit with an error)
        drop(self.tx);
        if let Err(err) = timeout(self.wait_timeout, async move { _ = self.rx.recv().await }).await
        {
            error!("task manager received error while waiting: {}", err);
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
