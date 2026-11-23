//! Job Priority System
//!
//! This module implements a priority-based job queue system that allows jobs to be
//! enqueued with different priority levels. Higher priority jobs are processed before
//! lower priority ones using a Weighted Fair Queuing (WFQ) algorithm to prevent
//! starvation of lower-priority background tasks under continuous high-priority load.

use serde::{Deserialize, Serialize};
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
        Some(self.cmp(other))
    }
}

impl<T> Ord for PriorityJob<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority (lower enum value) is considered greater for priority scheduling
        // If same priority, earlier enqueued jobs (smaller enqueued_at) come first
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.enqueued_at.cmp(&self.enqueued_at))
    }
}

/// Priority tier weights for Weighted Fair Queuing (WFQ).
/// Default distribution: 70% High (Critical/High), 20% Medium (Normal), 10% Low (Low/Background).
#[derive(Debug, Clone, Copy)]
pub struct WfqWeights {
    pub high_weight: usize,
    pub medium_weight: usize,
    pub low_weight: usize,
}

impl Default for WfqWeights {
    fn default() -> Self {
        Self {
            high_weight: 7,
            medium_weight: 2,
            low_weight: 1,
        }
    }
}

#[derive(Debug)]
struct QueueState<T> {
    /// Tier 0: Critical and High priority jobs
    high_queue: BinaryHeap<PriorityJob<T>>,
    /// Tier 1: Normal priority jobs
    medium_queue: BinaryHeap<PriorityJob<T>>,
    /// Tier 2: Low and Background priority jobs
    low_queue: BinaryHeap<PriorityJob<T>>,
    /// Current remaining quantum for WFQ selection
    quantum: [usize; 3],
    /// Configured weights for each tier
    weights: WfqWeights,
}

impl<T> QueueState<T> {
    fn new(weights: WfqWeights) -> Self {
        Self {
            high_queue: BinaryHeap::new(),
            medium_queue: BinaryHeap::new(),
            low_queue: BinaryHeap::new(),
            quantum: [
                weights.high_weight,
                weights.medium_weight,
                weights.low_weight,
            ],
            weights,
        }
    }

    fn reset_quantum(&mut self) {
        self.quantum = [
            self.weights.high_weight,
            self.weights.medium_weight,
            self.weights.low_weight,
        ];
    }

    fn is_empty(&self) -> bool {
        self.high_queue.is_empty() && self.medium_queue.is_empty() && self.low_queue.is_empty()
    }

    fn len(&self) -> usize {
        self.high_queue.len() + self.medium_queue.len() + self.low_queue.len()
    }
}

/// Priority queue implementation using Weighted Fair Queuing (WFQ)
/// to guarantee forward progress for lower priority jobs under sustained high-priority load.
#[derive(Debug, Clone)]
pub struct PriorityQueue<T> {
    state: Arc<Mutex<QueueState<T>>>,
}

impl<T> Default for PriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PriorityQueue<T> {
    /// Create a new priority queue with default WFQ weights (70% high, 20% medium, 10% low)
    pub fn new() -> Self {
        Self::with_weights(WfqWeights::default())
    }

    /// Create a new priority queue with custom WFQ weights
    pub fn with_weights(weights: WfqWeights) -> Self {
        Self {
            state: Arc::new(Mutex::new(QueueState::new(weights))),
        }
    }

    /// Enqueue a job with priority
    pub fn enqueue(&self, payload: T, priority: JobPriority) {
        let job = PriorityJob {
            payload,
            priority,
            enqueued_at: std::time::Instant::now(),
        };

        let mut state = self.state.lock().unwrap();
        match priority {
            JobPriority::Critical | JobPriority::High => {
                state.high_queue.push(job);
            }
            JobPriority::Normal => {
                state.medium_queue.push(job);
            }
            JobPriority::Low | JobPriority::Background => {
                state.low_queue.push(job);
            }
        }
        debug!("Enqueued job with priority: {:?}", priority);
    }

    /// Try to dequeue a job using Weighted Fair Queuing (WFQ)
    pub fn try_dequeue(&self) -> Option<PriorityJob<T>> {
        let mut state = self.state.lock().unwrap();
        if state.is_empty() {
            return None;
        }

        // Check if all non-empty queues have exhausted their active quantum.
        // If so, reset the quantum to initiate a fresh round.
        let has_eligible_queue = (state.quantum[0] > 0 && !state.high_queue.is_empty())
            || (state.quantum[1] > 0 && !state.medium_queue.is_empty())
            || (state.quantum[2] > 0 && !state.low_queue.is_empty());

        if !has_eligible_queue {
            state.reset_quantum();
        }

        // Try tier 0 (Critical / High)
        if state.quantum[0] > 0 && !state.high_queue.is_empty() {
            state.quantum[0] -= 1;
            return state.high_queue.pop();
        }

        // Try tier 1 (Normal / Medium)
        if state.quantum[1] > 0 && !state.medium_queue.is_empty() {
            state.quantum[1] -= 1;
            return state.medium_queue.pop();
        }

        // Try tier 2 (Low / Background)
        if state.quantum[2] > 0 && !state.low_queue.is_empty() {
            state.quantum[2] -= 1;
            return state.low_queue.pop();
        }

        // Fallback: pop from any available queue in priority order
        if let Some(job) = state.high_queue.pop() {
            Some(job)
        } else if let Some(job) = state.medium_queue.pop() {
            Some(job)
        } else {
            state.low_queue.pop()
        }
    }

    /// Dequeue strictly by priority order without weighted fair distribution
    pub fn try_dequeue_strict(&self) -> Option<PriorityJob<T>> {
        let mut state = self.state.lock().unwrap();
        if let Some(job) = state.high_queue.pop() {
            Some(job)
        } else if let Some(job) = state.medium_queue.pop() {
            Some(job)
        } else {
            state.low_queue.pop()
        }
    }

    /// Get the number of jobs in the queue
    pub fn len(&self) -> usize {
        let state = self.state.lock().unwrap();
        state.len()
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
                info!(priority = ?job.priority, "Processing priority job");
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
    fn test_wfq_prevents_low_priority_starvation() {
        let queue = PriorityQueue::<String>::new();

        // Enqueue 70 High jobs and 10 Low jobs
        for i in 0..70 {
            queue.enqueue(format!("high_{}", i), JobPriority::High);
        }
        for i in 0..10 {
            queue.enqueue(format!("low_{}", i), JobPriority::Low);
        }

        assert_eq!(queue.len(), 80);

        // Dequeue first 8 jobs: 7 should be High, then 8th MUST be Low (guaranteed forward progress)
        let mut high_count = 0;
        let mut low_count = 0;
        for _ in 0..8 {
            let job = queue.try_dequeue().unwrap();
            if job.priority == JobPriority::High {
                high_count += 1;
            } else if job.priority == JobPriority::Low {
                low_count += 1;
            }
        }

        assert_eq!(high_count, 7, "First 7 jobs should be High");
        assert_eq!(low_count, 1, "8th job must be Low, proving no starvation");
    }

    #[test]
    fn test_wfq_weighted_distribution() {
        let queue = PriorityQueue::<String>::new();

        // Enqueue 14 High, 4 Normal, 2 Low jobs (exact 7:2:1 ratio * 2 rounds)
        for i in 0..14 {
            queue.enqueue(format!("high_{}", i), JobPriority::High);
        }
        for i in 0..4 {
            queue.enqueue(format!("normal_{}", i), JobPriority::Normal);
        }
        for i in 0..2 {
            queue.enqueue(format!("low_{}", i), JobPriority::Low);
        }

        // First round of 10 jobs
        let mut round1_high = 0;
        let mut round1_norm = 0;
        let mut round1_low = 0;
        for _ in 0..10 {
            let job = queue.try_dequeue().unwrap();
            match job.priority {
                JobPriority::High => round1_high += 1,
                JobPriority::Normal => round1_norm += 1,
                JobPriority::Low => round1_low += 1,
                _ => {}
            }
        }
        assert_eq!(round1_high, 7);
        assert_eq!(round1_norm, 2);
        assert_eq!(round1_low, 1);

        // Second round of 10 jobs
        let mut round2_high = 0;
        let mut round2_norm = 0;
        let mut round2_low = 0;
        for _ in 0..10 {
            let job = queue.try_dequeue().unwrap();
            match job.priority {
                JobPriority::High => round2_high += 1,
                JobPriority::Normal => round2_norm += 1,
                JobPriority::Low => round2_low += 1,
                _ => {}
            }
        }
        assert_eq!(round2_high, 7);
        assert_eq!(round2_norm, 2);
        assert_eq!(round2_low, 1);

        assert!(queue.is_empty());
    }
}
