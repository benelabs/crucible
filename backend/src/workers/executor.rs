use crate::error::AppError;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::Instrument;

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
    /// Atomic counter for successfully completed tasks
    completed_count: Arc<AtomicU64>,
    /// Atomic counter for failed tasks
    failed_count: Arc<AtomicU64>,
}

impl TaskExecutor {
    /// Create a new TaskExecutor with the specified concurrency limit.
    ///
    /// # Arguments
    ///
    /// * `max_concurrency` - Maximum number of tasks that can run concurrently
    ///
    /// # Returns
    ///
    /// A new TaskExecutor instance
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            max_concurrency,
            running_count: Arc::new(AtomicU64::new(0)),
            completed_count: Arc::new(AtomicU64::new(0)),
            failed_count: Arc::new(AtomicU64::new(0)),
        }
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
        let completed_count = self.completed_count.clone();
        let failed_count = self.failed_count.clone();

        // Increment running count before attempting to acquire permit
        running_count.fetch_add(1, Ordering::Relaxed);

        // Spawn the task with tracing instrumentation
        tokio::spawn(
            async move {
                // Acquire semaphore permit (waits if necessary)
                let permit = semaphore.acquire_owned().await;

                // Execute the task
                let result = task.await;

                // Release the permit implicitly when it goes out of scope
                drop(permit);

                // Decrement running count
                running_count.fetch_sub(1, Ordering::Relaxed);

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

        // Wait until no tasks are running
        while self.running_count.load(Ordering::Relaxed) > 0 {
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
}
