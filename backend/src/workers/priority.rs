//! Job Priority System
//!
//! This module implements a priority-based job queue system that allows jobs to be
//! enqueued with different priority levels. Higher priority jobs are processed before
//! lower priority ones.

use serde::{Serialize, Deserialize};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

/// Priority levels for jobs
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    /// Highest priority - processed immediately when possible
    Critical = 0,
    /// High priority - processed before normal priority jobs
    High = 1,
    /// Normal/default priority
    Normal = 2,
    /// Low priority - processed after higher priority jobs
    Low = 3,
    /// Background priority - lowest priority, processed last
    Background = 4,
}

impl std::fmt::Display for JobPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobPriority::Critical => write!(f, "critical"),
            JobPriority::High => write!(f, "high"),
            JobPriority::Normal => write!(f, "normal"),
            JobPriority::Low => write!(f, "low"),
            JobPriority::Background => write!(f, "background"),
        }
    }
}

/// A job with priority information
#[derive(Debug, Clone)]
pub struct PriorityJob<T> {
    /// The actual job payload
    pub payload: T,
    /// Priority level of this job
    pub priority: JobPriority,
    /// Timestamp when job was enqueued
    pub enqueued_at: std::time::Instant,
}

impl<T> PartialEq for PriorityJob<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.enqueued_at == other.enqueued_at
    }
}

impl<T> Eq for PriorityJob<T> {}

impl<T> PartialOrd for PriorityJob<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Higher priority (lower enum value) comes first
        // If same priority, earlier enqueued jobs come first
        Some(
            self.priority
                .cmp(&other.priority)
                .then_with(|| other.enqueued_at.cmp(&self.enqueued_at)),
        )
    }
}

impl<T> Ord for PriorityJob<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// Priority queue implementation using BinaryHeap
#[derive(Debug, Clone)]
pub struct PriorityQueue<T> {
    heap: Arc<Mutex<BinaryHeap<PriorityJob<T>>>>,
}

impl<T> PriorityQueue<T> {
    /// Create a new priority queue
    pub fn new() -> Self {
        Self {
            heap: Arc::new(Mutex::new(BinaryHeap::new())),
        }
    }

    /// Enqueue a job with priority
    pub fn enqueue(&self, payload: T, priority: JobPriority) {
        let job = PriorityJob {
            payload,
            priority,
            enqueued_at: std::time::Instant::now(),
        };
        
        let mut heap = self.heap.lock().unwrap();
        heap.push(job);
        debug!("Enqueued job with priority: {:?}", priority);
    }

    /// Try to dequeue the highest priority job
    pub fn try_dequeue(&self) -> Option<PriorityJob<T>> {
        let mut heap = self.heap.lock().unwrap();
        heap.pop()
    }

    /// Get the number of jobs in the queue
    pub fn len(&self) -> usize {
        let heap = self.heap.lock().unwrap();
        heap.len()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Priority-aware job processor
#[derive(Debug)]
pub struct PriorityProcessor<T> {
    queue: PriorityQueue<T>,
    /// Channel for receiving jobs from external sources
    rx: mpsc::Receiver<PriorityJob<T>>,
}

impl<T: Send + 'static> PriorityProcessor<T> {
    /// Create a new priority processor
    pub fn new(queue: PriorityQueue<T>, rx: mpsc::Receiver<PriorityJob<T>>) -> Self {
        Self { queue, rx }
    }

    /// Start processing jobs from the queue
    pub async fn start(mut self) {
        info!("Starting priority job processor");
        
        loop {
            // First try to get a job from our priority queue
            if let Some(job) = self.queue.try_dequeue() {
                info!(priority = ?job.priority, "Processing high-priority job");
                // Process the job here
                self.process_job(job).await;
                continue;
            }
            
            // If no priority job available, try to receive from channel
            match self.rx.recv().await {
                Some(job) => {
                    info!(priority = ?job.priority, "Processing job from channel");
                    self.process_job(job).await;
                }
                None => {
                    error!("Job receiver channel closed");
                    break;
                }
            }
        }
    }

    /// Process a single job
    async fn process_job(&self, job: PriorityJob<T>) {
        // This is where the actual job processing logic would go
        // For now, just log it
        debug!(
            priority = ?job.priority,
            enqueued_at = ?job.enqueued_at.elapsed(),
            "Processing job"
        );
        
        // Simulate some async work
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    
    #[test]
    fn test_priority_ordering() {
        let job1 = PriorityJob {
            payload: "job1",
            priority: JobPriority::Critical,
            enqueued_at: std::time::Instant::now(),
        };
        
        let job2 = PriorityJob {
            payload: "job2",
            priority: JobPriority::High,
            enqueued_at: std::time::Instant::now(),
        };
        
        let job3 = PriorityJob {
            payload: "job3",
            priority: JobPriority::Normal,
            enqueued_at: std::time::Instant::now(),
        };
        
        // Critical should come before High, which should come before Normal
        assert!(job1 > job2);
        assert!(job2 > job3);
        assert!(job1 > job3);
    }
    
    #[test]
    fn test_same_priority_ordering() {
        let now = std::time::Instant::now();
        let later = now + Duration::from_millis(1);
        
        let job1 = PriorityJob {
            payload: "job1",
            priority: JobPriority::Normal,
            enqueued_at: now,
        };
        
        let job2 = PriorityJob {
            payload: "job2",
            priority: JobPriority::Normal,
            enqueued_at: later,
        };
        
        // Earlier enqueued jobs should come first
        assert!(job1 > job2);
    }
    
    #[test]
    fn test_priority_queue_basic() {
        let queue = PriorityQueue::<String>::new();
        
        // Enqueue jobs in different order
        queue.enqueue("job1".to_string(), JobPriority::Normal);
        queue.enqueue("job2".to_string(), JobPriority::Critical);
        queue.enqueue("job3".to_string(), JobPriority::High);
        
        // Should get critical first
        let job = queue.try_dequeue().unwrap();
        assert_eq!(job.payload, "job2");
        assert_eq!(job.priority, JobPriority::Critical);
        
        // Then high
        let job = queue.try_dequeue().unwrap();
        assert_eq!(job.payload, "job3");
        assert_eq!(job.priority, JobPriority::High);
        
        // Then normal
        let job = queue.try_dequeue().unwrap();
        assert_eq!(job.payload, "job1");
        assert_eq!(job.priority, JobPriority::Normal);
    }
}