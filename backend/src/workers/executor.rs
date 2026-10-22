use crate::error::AppError;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::Instrument;

/// Default task execution timeout (5 minutes)
pub const DEFAULT_JOB_TIMEOUT: Duration = Duration::from_secs(300);

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
    /// Atomic counter for tasks currently executing (holding permits)
    running_count: Arc<AtomicU64>,
    /// Atomic counter for tasks waiting for permits
    queued_count: Arc<AtomicU64>,
    /// Atomic counter for successfully completed tasks
    completed_count: Arc<AtomicU64>,
    /// Atomic counter for failed tasks
    failed_count: Arc<AtomicU64>,
    /// Configurable per-task timeout deadline
    job_timeout: Duration,
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
        Self::new_with_timeout(max_concurrency, DEFAULT_JOB_TIMEOUT)
    }

    /// Create a new TaskExecutor with a specified concurrency limit and custom per-task timeout.
    pub fn new_with_timeout(max_concurrency: usize, job_timeout: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            max_concurrency,
            running_count: Arc::new(AtomicU64::new(0)),
            queued_count: Arc::new(AtomicU64::new(0)),
            completed_count: Arc::new(AtomicU64::new(0)),
            failed_count: Arc::new(AtomicU64::new(0)),
            job_timeout,
        }
    }

    /// Set a custom task timeout on this executor
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.job_timeout = timeout;
        self
    }

    /// Get configured job timeout
    pub fn job_timeout(&self) -> Duration {
        self.job_timeout
    }

    /// Get the current number of running tasks (holding concurrency permits).
    ///
    /// # Returns
    ///
    /// Current running task count
    pub fn running_count(&self) -> u64 {
        self.running_count.load(Ordering::Relaxed)
    }

    /// Get the current number of queued tasks (waiting for concurrency permits).
    pub fn queued_count(&self) -> u64 {
        self.queued_count.load(Ordering::Relaxed)
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
        // Clone the semaphore, counters, and timeout for the task closure
        let semaphore = self.semaphore.clone();
        let running_count = self.running_count.clone();
        let queued_count = self.queued_count.clone();
        let completed_count = self.completed_count.clone();
        let failed_count = self.failed_count.clone();
        let job_timeout = self.job_timeout;

        // Increment queued count before attempting to acquire permit
        queued_count.fetch_add(1, Ordering::Relaxed);

        // Spawn the task with tracing instrumentation
        tokio::spawn(
            async move {
                // Acquire semaphore permit (waits if necessary)
                let permit = semaphore.acquire_owned().await;
                queued_count.fetch_sub(1, Ordering::Relaxed);
                running_count.fetch_add(1, Ordering::Relaxed);

                // Execute the task with per-job timeout enforcement
                let result = match tokio::time::timeout(job_timeout, task).await {
                    Ok(res) => res,
                    Err(_) => {
                        tracing::error!(
                            timeout_secs = job_timeout.as_secs(),
                            "Task timed out after exceeding execution deadline"
                        );
                        Err(AppError::InternalError(format!(
                            "Task execution timed out after {:?}",
                            job_timeout
                        )))
                    }
                };

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

        // Wait until no tasks are running or queued
        while self.running_count.load(Ordering::Relaxed) > 0
            || self.queued_count.load(Ordering::Relaxed) > 0
        {
            tracing::debug!(
                running = self.running_count(),
                queued = self.queued_count(),
                "Still waiting for tasks to complete"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
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
    async fn test_executor_task_timeout() {
        // Create an executor with a very short timeout (50ms)
        let executor = TaskExecutor::new_with_timeout(2, Duration::from_millis(50));

        // Start a task that hangs longer than the timeout (300ms)
        let handle = executor.execute(Box::pin(async move {
            sleep(Duration::from_millis(300)).await;
            Ok(())
        }));

        let result = handle.await.unwrap();
        assert!(result.is_err(), "Task should fail due to timeout");
        assert_eq!(executor.failed_count(), 1);
        assert_eq!(executor.running_count(), 0);

        // Verify slot was released: we can execute a new task successfully
        let fast_handle = executor.execute(Box::pin(async move { Ok(()) }));
        let fast_result = fast_handle.await.unwrap();
        assert!(fast_result.is_ok());
        assert_eq!(executor.completed_count(), 1);
    }

    #[tokio::test]
    async fn test_executor_timeout_configuration() {
        let executor = TaskExecutor::new(4);
        assert_eq!(executor.job_timeout(), DEFAULT_JOB_TIMEOUT);

        let customized = executor.with_timeout(Duration::from_secs(45));
        assert_eq!(customized.job_timeout(), Duration::from_secs(45));
    }
}
