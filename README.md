# tokio-task-manager

A crate which provides sync primitives with as main goal
to allow an async application making use of [the Tokio runtime](https://tokio.rs/)
to be able to gracefully shutdown, meaning that ideally the server process exits
only when all active tasks were done with their ongoing work.

[![Crates.io][crates-badge]][crates-url]
[![Docs.rs][docs-badge]][docs-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/tokio-task-manager.svg
[crates-url]: https://crates.io/crates/tokio-task-manager
[docs-badge]: https://img.shields.io/docsrs/tokio-task-manager/latest
[docs-url]: https://docs.rs/tokio-task-manager/latest/tokio_task_manager/index.html
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/OTA-Insight/tokio-task-manager/blob/master/LICENSE
[actions-badge]: https://github.com/OTA-Insight/tokio-task-manager/workflows/CI/badge.svg
[actions-url]: https://github.com/OTA-Insight/tokio-task-manager/actions?query=workflow%3ACI+branch%main

## Rust versions supported

[v1.61.0](https://blog.rust-lang.org/2022/05/19/Rust-1.61.0.html) and above,
language edition `2021`.

## Example

```rust
use std::time::Duration;
use tokio_task_manager::TaskManager;

#[tokio::main]
async fn main() {
    // An application requires only a single TaskManager,
    // created usually where you also build and start the Tokio runtime.
    let tm = TaskManager::new(Duration::from_millis(200));

    // In this example we spawn 10 tasks,
    // where each of them also spawns a task themselves,
    // resulting in a total of 20 tasks which we'll want to wait for.
    let (tx, mut rx) = tokio::sync::mpsc::channel(20);
    for i in 0..10 {
        let tx = tx.clone();
        let n = i;

        // create a task per task that we spawn, such that:
        // - the application can wait until the task is dropped,
        //   identifying the spawned task is finished;
        // - the spawn task knows that the application is gracefully shutting down (.wait);
        let mut task = tm.task();
        tokio::spawn(async move {
            // spawn also child task to test task cloning,
            // a task is typically cloned for tasks within tasks,
            // each cloned task also needs to be dropped prior to
            // the application being able to gracefully shut down.
            let mut child_task = task.clone();
            let child_tx = tx.clone();
            let m = n;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(m * 10)).await;
                // Using the tokio::select! macro you can allow a task
                // to either get to its desired work, or quit already
                // in case the application is planning to shut down.
                //
                // A typical use case of this is a server which is waiting
                // for an incoming request, which is a text-book example
                // of a task in idle state.
                tokio::select! {
                    result = child_tx.send((m+1)*10) => assert!(result.is_ok()),
                    _ = child_task.wait() => (),
                }
            });
            // Do the actual work.
            tokio::time::sleep(Duration::from_millis(n * 10)).await;
            tokio::select! {
                result = tx.send(n) => assert!(result.is_ok()),
                _ = task.wait() => (),
            }
        });
    }

    // we also create a task for something that will never finish,
    // just to show that the tokio::select! approach does work...
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

    // sleep for 100ms, just to ensure that all child tasks have been spawned as well
    tokio::time::sleep(Duration::from_millis(100)).await;

    // collect all results
    drop(tx);

    // notify all spawned tasks that we wish to gracefully shut down
    // and wait until they do. The resulting boolean is true if the
    // waiting terminated gracefully (meaning all tasks quit on their own while they were idle).
    assert!(tm.wait().await);

    // collect all our results,
    // which we can do all at once given the channel
    // was created with a sufficient buffer size.
    let mut results = Vec::with_capacity(20);
    while let Some(n) = rx.recv().await {
        results.push(n);
    }

    // test to proof we received all expected results
    results.sort_unstable();
    assert_eq!(
        &results,
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
    );
}
```
