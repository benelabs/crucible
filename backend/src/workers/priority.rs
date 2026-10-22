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
        // Higher priority (lower enum value) is Greater
        // If same priority, earlier enqueued jobs are Greater
        Some(
            other
                .priority
                .cmp(&self.priority)
                .then_with(|| other.enqueued_at.cmp(&self.enqueued_at)),
        )
    }
}

impl<T> Ord for PriorityJob<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

use std::collections::VecDeque;

/// Default weights for weighted fair queuing: High (70%), Normal/Medium (20%), Low (10%), Background (10%)
pub const DEFAULT_HIGH_WEIGHT: usize = 7;
pub const DEFAULT_NORMAL_WEIGHT: usize = 2;
pub const DEFAULT_LOW_WEIGHT: usize = 1;
pub const DEFAULT_BACKGROUND_WEIGHT: usize = 1;

/// Internal state for weighted fair queuing across priority queues
#[derive(Debug)]
struct WfqScheduler<T> {
    critical: VecDeque<PriorityJob<T>>,
    high: VecDeque<PriorityJob<T>>,
    normal: VecDeque<PriorityJob<T>>,
    low: VecDeque<PriorityJob<T>>,
    background: VecDeque<PriorityJob<T>>,
    weights: [usize; 4],
    current_tier_idx: usize,
    tier_quota_remaining: usize,
}

impl<T> WfqScheduler<T> {
    fn new(weights: [usize; 4]) -> Self {
        let initial_quota = weights[0].max(1);
        Self {
            critical: VecDeque::new(),
            high: VecDeque::new(),
            normal: VecDeque::new(),
            low: VecDeque::new(),
            background: VecDeque::new(),
            weights,
            current_tier_idx: 0,
            tier_quota_remaining: initial_quota,
        }
    }

    fn enqueue(&mut self, job: PriorityJob<T>) {
        match job.priority {
            JobPriority::Critical => self.critical.push_back(job),
            JobPriority::High => self.high.push_back(job),
            JobPriority::Normal => self.normal.push_back(job),
            JobPriority::Low => self.low.push_back(job),
            JobPriority::Background => self.background.push_back(job),
        }
    }

    fn try_dequeue(&mut self) -> Option<PriorityJob<T>> {
        // Critical priority jobs are always processed immediately when present
        if let Some(job) = self.critical.pop_front() {
            return Some(job);
        }

        let non_critical_total =
            self.high.len() + self.normal.len() + self.low.len() + self.background.len();
        if non_critical_total == 0 {
            return None;
        }

        // Try across the 4 weighted tiers using Weighted Fair Queuing (WFQ)
        for _ in 0..4 {
            if self.tier_quota_remaining == 0 {
                self.current_tier_idx = (self.current_tier_idx + 1) % 4;
                self.tier_quota_remaining = self.weights[self.current_tier_idx].max(1);
            }

            let job_opt = match self.current_tier_idx {
                0 => self.high.pop_front(),
                1 => self.normal.pop_front(),
                2 => self.low.pop_front(),
                3 => self.background.pop_front(),
                _ => unreachable!(),
            };

            if let Some(job) = job_opt {
                self.tier_quota_remaining -= 1;
                if self.tier_quota_remaining == 0 {
                    self.current_tier_idx = (self.current_tier_idx + 1) % 4;
                    self.tier_quota_remaining = self.weights[self.current_tier_idx].max(1);
                }
                return Some(job);
            } else {
                // Tier is empty: advance to next tier without consuming quota
                self.current_tier_idx = (self.current_tier_idx + 1) % 4;
                self.tier_quota_remaining = self.weights[self.current_tier_idx].max(1);
            }
        }

        None
    }

    fn len(&self) -> usize {
        self.critical.len()
            + self.high.len()
            + self.normal.len()
            + self.low.len()
            + self.background.len()
    }
}

/// Priority queue implementation using Weighted Fair Queuing (WFQ)
///
/// Prevents starvation of medium, low, and background tasks under sustained
/// high-priority workloads by serving queues according to configurable weights
/// (default: 70% High, 20% Normal, 10% Low, 10% Background). Critical priority
/// tasks are always expedited immediately when present.
#[derive(Debug, Clone)]
pub struct PriorityQueue<T> {
    scheduler: Arc<Mutex<WfqScheduler<T>>>,
}

impl<T> PriorityQueue<T> {
    /// Create a new priority queue with default Weighted Fair Queuing (7:2:1:1)
    pub fn new() -> Self {
        Self::new_with_weights(
            DEFAULT_HIGH_WEIGHT,
            DEFAULT_NORMAL_WEIGHT,
            DEFAULT_LOW_WEIGHT,
            DEFAULT_BACKGROUND_WEIGHT,
        )
    }

    /// Create a priority queue with custom tier weights for [high, normal, low, background]
    pub fn new_with_weights(
        high_weight: usize,
        normal_weight: usize,
        low_weight: usize,
        background_weight: usize,
    ) -> Self {
        let weights = [
            high_weight.max(1),
            normal_weight.max(1),
            low_weight.max(1),
            background_weight.max(1),
        ];
        Self {
            scheduler: Arc::new(Mutex::new(WfqScheduler::new(weights))),
        }
    }

    /// Enqueue a job with priority
    pub fn enqueue(&self, payload: T, priority: JobPriority) {
        let job = PriorityJob {
            payload,
            priority,
            enqueued_at: std::time::Instant::now(),
        };

        let mut scheduler = self.scheduler.lock().unwrap();
        scheduler.enqueue(job);
        debug!("Enqueued job with priority: {:?}", priority);
    }

    /// Try to dequeue the next job according to Weighted Fair Queuing
    pub fn try_dequeue(&self) -> Option<PriorityJob<T>> {
        let mut scheduler = self.scheduler.lock().unwrap();
        scheduler.try_dequeue()
    }

    /// Get the total number of jobs across all queues
    pub fn len(&self) -> usize {
        let scheduler = self.scheduler.lock().unwrap();
        scheduler.len()
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

    #[test]
    fn test_wfq_low_priority_starvation_prevention() {
        let queue = PriorityQueue::<String>::new();

        // Enqueue 20 High priority jobs and 2 Low priority jobs
        for i in 0..20 {
            queue.enqueue(format!("high-{}", i), JobPriority::High);
        }
        queue.enqueue("low-0".to_string(), JobPriority::Low);
        queue.enqueue("low-1".to_string(), JobPriority::Low);

        let mut dequeued = Vec::new();
        while let Some(job) = queue.try_dequeue() {
            dequeued.push((job.payload, job.priority));
        }

        assert_eq!(dequeued.len(), 22);

        // Under 7:2:1:1 WFQ, the first 7 jobs should be High,
        // and the 8th job must be Low (since Normal is empty),
        // proving Low priority is NOT starved under continuous High load!
        for i in 0..7 {
            assert_eq!(dequeued[i].1, JobPriority::High);
        }
        assert_eq!(dequeued[7].0, "low-0");
        assert_eq!(dequeued[7].1, JobPriority::Low);
    }

    #[test]
    fn test_wfq_custom_weights() {
        // High: 3, Normal: 1, Low: 1, Background: 1
        let queue = PriorityQueue::<String>::new_with_weights(3, 1, 1, 1);

        for i in 0..6 {
            queue.enqueue(format!("high-{}", i), JobPriority::High);
        }
        for i in 0..2 {
            queue.enqueue(format!("normal-{}", i), JobPriority::Normal);
        }

        // First 3 should be High
        for _ in 0..3 {
            assert_eq!(queue.try_dequeue().unwrap().priority, JobPriority::High);
        }
        // Next 1 should be Normal
        assert_eq!(queue.try_dequeue().unwrap().priority, JobPriority::Normal);
        // Next 3 should be High
        for _ in 0..3 {
            assert_eq!(queue.try_dequeue().unwrap().priority, JobPriority::High);
        }
        // Next 1 should be Normal
        assert_eq!(queue.try_dequeue().unwrap().priority, JobPriority::Normal);
    }
}