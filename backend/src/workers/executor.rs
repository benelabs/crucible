use crate::error::AppError;
use futures_util::FutureExt;
use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::{JoinError, JoinHandle};
use tracing::Instrument;

/// Extract string message from a panic payload.
pub fn extract_panic_message(err: &(dyn Any + Send + 'static)) -> String {
    if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic payload".to_string()
    }
}

/// Handle a Tokio JoinError, extracting the panic message if the task panicked.
pub fn handle_join_error(err: JoinError) -> AppError {
    if err.is_panic() {
        let panic_payload = err.into_panic();
        let msg = extract_panic_message(&*panic_payload);
        tracing::error!(panic = %msg, "Background task failed due to panic");
        AppError::InternalError(format!("Task panicked: {}", msg))
    } else {
        tracing::error!(error = %err, "Background task failed to join");
        AppError::InternalError(format!("Task failed to join: {}", err))
    }
}

/// A concurrent task executor that limits the number of concurrently executing tasks.
///
/// This executor uses a semaphore to limit concurrency and tracks metrics for
/// running, completed, and failed tasks. Tasks are spawned as Tokio tasks and
/// the executor can wait for all tasks to complete during shutdown.
///
/// # Example
///
/// ```
/// use crucible_backend::workers::executor::TaskExecutor;
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() {
///     let executor = TaskExecutor::new(5); // Limit to 5 concurrent tasks
///     
///     // Execute some tasks
///     executor.execute(Box::pin(async move {
///         // Task logic here
///         Ok(())
///     }));
///     
///     // Wait for all tasks to complete
///     executor.shutdown().await;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct TaskExecutor {
    /// Semaphore to limit concurrent task execution
    semaphore: Arc<Semaphore>,
    /// Maximum concurrency limit
    max_concurrency: usize,
    /// Atomic counter for currently running tasks
    running_count: Arc<AtomicU64>,
    /// Atomic counter for tasks that were submitted but have not finished yet.
    /// Includes tasks still waiting on a concurrency permit.
    pending_count: Arc<AtomicU64>,
    /// Atomic counter for successfully completed tasks
    completed_count: Arc<AtomicU64>,
    /// Atomic counter for failed tasks
    failed_count: Arc<AtomicU64>,
    /// Per-task execution timeout deadline (default: 5 minutes)
    task_timeout: Duration,
}

impl TaskExecutor {
    /// Create a new TaskExecutor with the specified concurrency limit and default 5-minute timeout.
    ///
    /// # Arguments
    ///
    /// * `max_concurrency` - Maximum number of tasks that can run concurrently
    ///
    /// # Returns
    ///
    /// A new TaskExecutor instance
    pub fn new(max_concurrency: usize) -> Self {
        Self::with_timeout(max_concurrency, Duration::from_secs(300))
    }

    /// Create a new TaskExecutor with specified concurrency limit and custom task timeout.
    pub fn with_timeout(max_concurrency: usize, task_timeout: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            max_concurrency,
            running_count: Arc::new(AtomicU64::new(0)),
            pending_count: Arc::new(AtomicU64::new(0)),
            completed_count: Arc::new(AtomicU64::new(0)),
            failed_count: Arc::new(AtomicU64::new(0)),
            task_timeout,
        }
    }

    /// Returns the configured per-task execution timeout.
    pub fn task_timeout(&self) -> Duration {
        self.task_timeout
    }

    /// Get the current number of running tasks.
    ///
    /// # Returns
    ///
    /// Current running task count
    pub fn running_count(&self) -> u64 {
        self.running_count.load(Ordering::Relaxed)
    }

    /// Get the number of successfully completed tasks.
    ///
    /// # Returns
    ///
    /// Completed task count
    pub fn completed_count(&self) -> u64 {
        self.completed_count.load(Ordering::Relaxed)
    }

    /// Get the number of failed tasks.
    ///
    /// # Returns
    ///
    /// Failed task count
    pub fn failed_count(&self) -> u64 {
        self.failed_count.load(Ordering::Relaxed)
    }

    /// Execute an asynchronous task with concurrency limiting.
    ///
    /// This method acquires a semaphore permit before spawning the task.
    /// If the permit cannot be acquired immediately, the task will wait
    /// until a permit becomes available.
    ///
    /// # Arguments
    ///
    /// * `task` - A boxed async future that returns Result<(), AppError>
    ///
    /// # Returns
    ///
    /// A JoinHandle that can be used to await the task completion
    #[tracing::instrument(skip(self, task), fields(executor.max_concurrency = self.max_concurrency))]
    pub fn execute(
        &self,
        task: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + 'static>,
        >,
    ) -> JoinHandle<Result<(), AppError>> {
        // Clone the semaphore and counters for the task closure
        let semaphore = self.semaphore.clone();
        let running_count = self.running_count.clone();
        let pending_count = self.pending_count.clone();
        let completed_count = self.completed_count.clone();
        let failed_count = self.failed_count.clone();
        let task_timeout = self.task_timeout;

        // Submitted tasks count as pending right away, even while they wait for a permit,
        // so shutdown cannot return while a spawned task is still queued.
        pending_count.fetch_add(1, Ordering::Relaxed);

        // Spawn the task with tracing instrumentation
        tokio::spawn(
            async move {
                // Acquire semaphore permit (waits if necessary)
                let permit = semaphore.acquire_owned().await;

                // Only tasks holding a permit are counted as running
                running_count.fetch_add(1, Ordering::Relaxed);

                // Catch task panics and enforce a bounded timeout.
                let task_with_catch = AssertUnwindSafe(task).catch_unwind();

                let result = match tokio::time::timeout(task_timeout, task_with_catch).await {
                    Ok(Ok(task_res)) => task_res,
                    Ok(Err(panic_payload)) => {
                        let msg = extract_panic_message(&*panic_payload);
                        tracing::error!(
                            panic_message = %msg,
                            "Background task panicked during execution"
                        );
                        Err(AppError::InternalError(format!("Task panicked: {}", msg)))
                    }
                    Err(_) => {
                        tracing::error!(
                            timeout_secs = task_timeout.as_secs(),
                            "Task execution exceeded timeout deadline"
                        );
                        Err(AppError::InternalError(format!(
                            "Task execution timed out after {:?}",
                            task_timeout
                        )))
                    }
                };

                // Release the permit implicitly when it goes out of scope
                drop(permit);

                // Decrement running count
                running_count.fetch_sub(1, Ordering::Relaxed);

                // Task is no longer pending
                pending_count.fetch_sub(1, Ordering::Relaxed);

                // Update appropriate counter based on result
                match result {
                    Ok(()) => {
                        completed_count.fetch_add(1, Ordering::Relaxed);
                        tracing::info!("Task completed successfully");
                        Ok(())
                    }
                    Err(e) => {
                        failed_count.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(error = %e, "Task failed");
                        Err(e)
                    }
                }
            }
            .instrument(tracing::info_span!("executor_task")),
        )
    }

    /// Wait for all currently executing tasks to complete.
    ///
    /// This method waits until the running count reaches zero, indicating
    /// that all tasks that were executing when this method was called
    /// have completed. It does not prevent new tasks from being started
    /// after this method is called.
    ///
    /// # Returns
    ///
    /// A future that resolves when all tasks have completed
    #[tracing::instrument(skip(self))]
    pub async fn shutdown(&self) {
        tracing::info!(
            running = self.running_count(),
            completed = self.completed_count(),
            failed = self.failed_count(),
            "Waiting for executor tasks to complete"
        );

        // Wait until no task is still outstanding
        while self.pending_count.load(Ordering::Relaxed) > 0 {
            tracing::debug!(
                running = self.running_count(),
                "Still waiting for tasks to complete"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        tracing::info!("All executor tasks have completed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_executor_creation() {
        let executor = TaskExecutor::new(3);
        assert_eq!(executor.running_count(), 0);
        assert_eq!(executor.completed_count(), 0);
        assert_eq!(executor.failed_count(), 0);
        assert_eq!(executor.max_concurrency, 3);
    }

    #[tokio::test]
    async fn test_executor_concurrency_limit() {
        let executor = TaskExecutor::new(2);
        assert_eq!(executor.max_concurrency, 2);

        // Execute 4 tasks that each take some time
        let mut handles = Vec::new();

        for i in 0..4 {
            let executor = executor.clone();
            let handle = executor.execute(Box::pin(async move {
                sleep(Duration::from_millis(100)).await;
                Ok(())
            }));
            handles.push(handle);
        }

        // Wait a bit and check that running count never exceeds limit
        sleep(Duration::from_millis(50)).await;
        assert!(executor.running_count() <= 2);

        // Wait for all tasks to complete
        for handle in handles {
            let _ = handle.await;
        }

        // Check final counts
        assert_eq!(executor.running_count(), 0);
        assert_eq!(executor.completed_count(), 4);
        assert_eq!(executor.failed_count(), 0);
    }

    #[tokio::test]
    async fn test_executor_task_success_failure() {
        let executor = TaskExecutor::new(5);

        // Execute a successful task
        let success_handle = executor.execute(Box::pin(async move { Ok(()) }));

        // Execute a failing task
        let fail_handle = executor.execute(Box::pin(async move { Err(AppError::Internal) }));

        // Wait for both to complete
        let success_result = success_handle.await.unwrap();
        let fail_result = fail_handle.await.unwrap();

        assert!(success_result.is_ok());
        assert!(fail_result.is_err());

        // Check counts
        assert_eq!(executor.running_count(), 0);
        assert_eq!(executor.completed_count(), 1);
        assert_eq!(executor.failed_count(), 1);
    }

    #[tokio::test]
    async fn test_executor_shutdown_waits_for_tasks() {
        let executor = TaskExecutor::new(2);

        // Start some long-running tasks
        let mut handles = Vec::new();
        for i in 0..3 {
            let executor = executor.clone();
            let handle = executor.execute(Box::pin(async move {
                sleep(Duration::from_millis(200)).await;
                Ok(())
            }));
            handles.push(handle);
        }

        // Immediately call shutdown - it should wait for tasks to complete
        let executor_shutdown = executor.clone();
        let shutdown_handle = tokio::spawn(async move {
            executor_shutdown.shutdown().await;
        });

        // Give shutdown a moment to start
        sleep(Duration::from_millis(50)).await;

        // Shutdown should still be waiting (tasks still running)
        assert!(executor.running_count() > 0);

        // Wait for shutdown to complete
        let _ = shutdown_handle.await;

        // After shutdown, no tasks should be running
        assert_eq!(executor.running_count(), 0);
        assert_eq!(executor.completed_count(), 3);
        assert_eq!(executor.failed_count(), 0);
    }

    #[tokio::test]
    async fn test_executor_catches_panics_with_diagnostic_trace() {
        let executor = TaskExecutor::new(2);

        // Execute task that panics with &str
        let handle_str = executor.execute(Box::pin(async {
            panic!("critical database invariant failed");
            #[allow(unreachable_code)]
            Ok(())
        }));

        let res_str = handle_str.await.unwrap();
        assert!(res_str.is_err(), "Panicking task must return error");
        let err_msg = format!("{}", res_str.unwrap_err());
        assert!(
            err_msg.contains("critical database invariant failed"),
            "Panic payload must be preserved: {}",
            err_msg
        );
        assert_eq!(executor.failed_count(), 1);
        assert_eq!(executor.running_count(), 0);

        // Execute task that panics with String
        let handle_string = executor.execute(Box::pin(async {
            panic!("{}", format!("out of memory at item {}", 100));
            #[allow(unreachable_code)]
            Ok(())
        }));

        let res_string = handle_string.await.unwrap();
        assert!(res_string.is_err());
        let err_msg2 = format!("{}", res_string.unwrap_err());
        assert!(
            err_msg2.contains("out of memory at item 100"),
            "Formatted panic payload must be preserved: {}",
            err_msg2
        );
        assert_eq!(executor.failed_count(), 2);
        assert_eq!(executor.running_count(), 0);
    }

    #[tokio::test]
    async fn test_executor_permits_released_after_panic() {
        let executor = TaskExecutor::new(1);

        let panic_handle = executor.execute(Box::pin(async {
            panic!("worker thread failed");
            #[allow(unreachable_code)]
            Ok(())
        }));
        let _ = panic_handle.await.unwrap();

        // Concurrency slot must be immediately freed and available
        let success_handle = executor.execute(Box::pin(async { Ok(()) }));
        let res = success_handle.await.unwrap();
        assert!(
            res.is_ok(),
            "Subsequent task must succeed without slot leak"
        );
        assert_eq!(executor.completed_count(), 1);
        assert_eq!(executor.failed_count(), 1);
        assert_eq!(executor.running_count(), 0);
    }

    #[tokio::test]
    async fn test_handle_join_error_extracts_panic_payload() {
        let raw_handle: JoinHandle<()> = tokio::spawn(async {
            panic!("fatal worker panic");
        });
        let join_err = raw_handle.await.unwrap_err();
        let app_err = handle_join_error(join_err);
        let err_str = format!("{}", app_err);
        assert!(err_str.contains("fatal worker panic"));
    }

    #[tokio::test]
    async fn test_executor_default_timeout() {
        let executor = TaskExecutor::new(4);
        assert_eq!(executor.task_timeout(), Duration::from_secs(300));
    }

    #[tokio::test]
    async fn test_executor_custom_timeout_and_deadline_enforcement() {
        // Limit concurrency to 1 and timeout to 50ms
        let executor = TaskExecutor::with_timeout(1, Duration::from_millis(50));
        assert_eq!(executor.task_timeout(), Duration::from_millis(50));

        // Spawn a task that sleeps longer than timeout (2 seconds)
        let stuck_handle = executor.execute(Box::pin(async {
            sleep(Duration::from_secs(2)).await;
            Ok(())
        }));

        let stuck_res = stuck_handle.await.unwrap();
        assert!(stuck_res.is_err(), "Stuck task must fail due to timeout");
        assert_eq!(executor.failed_count(), 1);
        assert_eq!(executor.running_count(), 0);

        // Verify concurrency slot is released and not leaked
        let next_handle = executor.execute(Box::pin(async { Ok(()) }));
        let next_res = next_handle.await.unwrap();
        assert!(
            next_res.is_ok(),
            "Subsequent task must succeed without slot leak"
        );
        assert_eq!(executor.completed_count(), 1);
        assert_eq!(executor.running_count(), 0);
    }
}
