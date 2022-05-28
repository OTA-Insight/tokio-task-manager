use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::copy;
use tokio::net::{TcpListener, TcpStream};
use tokio_task_manager::{Task, TaskManager};

#[tokio::main]
async fn main() {
    // An application requires only a single TaskManager,
    // created usually where you also build and start the Tokio runtime.
    let tm = TaskManager::new(Duration::from_secs(10));

    // create our listener and give it our task,
    // each socket handle will also
    let listener = TcpListener::bind("127.0.0.1:4000").await.unwrap();
    let mut task = tm.task();
    tokio::spawn(async move {
        loop {
            let (stream, addr) = tokio::select! {
                r = listener.accept() => match r {
                    Err(_) => continue,
                    Ok(s) => s,
                },
                // the loop will exit in case the loop is waiting for a
                // new incoming request and the task manager has signalled
                // that the tasks are to gracefully shutdown
                _ = task.wait() => {
                    return;
                }
            };
            // create a new task for each socket handle, as to ensure
            // the application is blocked until all open tasks are closed
            let task = task.clone();
            tokio::spawn(async move {
                handle(task, stream, addr).await;
            });
        }
    });

    tm.shutdown_gracefully_on_ctrl_c().await;
}

// We do not use the task here,
// meaning the task manager cannot signal to this task it is to gracefully shutdown,
// but it will mean that the graceful shutdown process is blocked until also
// this task is dropped.
async fn handle(_task: Task, mut stream: TcpStream, addr: SocketAddr) {
    let (mut reader, mut writer) = stream.split();
    match copy(&mut reader, &mut writer).await {
        Ok(amt) => println!("handle: wrote {} bytes to {}", amt, addr),
        Err(e) => println!("handle: error on {}: {}", addr, e),
    };
}
