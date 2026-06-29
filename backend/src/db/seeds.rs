//! Database seed utilities for development and testing environments.
//!
//! This module provides functions to populate the database with well-known
//! baseline data so that the application can be exercised without manual
//! setup. Seeds are idempotent – running them multiple times is safe.
//!
//! # Example
//! ```rust,no_run
//! use sqlx::PgPool;
//! use backend::db::seeds::run_all;
//!
//! # async fn example(pool: &PgPool) -> anyhow::Result<()> {
//! run_all(pool).await?;
//! # Ok(())
//! # }
//! ```

#![allow(dead_code)]

use chrono::Utc;
use sqlx::PgPool;
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur while seeding the database.
#[derive(Debug, Error)]
pub enum SeedError {
    /// A SQLx database error occurred.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// A seed step failed for a domain-specific reason.
    #[error("Seed step failed ({step}): {reason}")]
    StepFailed {
        /// Name of the seed step that failed.
        step: String,
        /// Human-readable reason.
        reason: String,
    },
    /// Multiple seed errors collected while running multiple steps.
    #[error("Multiple seed errors: {0:?}")]
    Multiple(Vec<SeedError>),
}

// ---------------------------------------------------------------------------
// Seed data definitions
// ---------------------------------------------------------------------------

/// A minimal user record used for seeding.
#[derive(Debug, Clone)]
pub struct SeedUser {
    pub id: Uuid,
    pub username: String,
    pub email: String,
}

/// A minimal feature-flag record used for seeding.
#[derive(Debug, Clone)]
pub struct SeedFlag {
    pub key: String,
    pub enabled: bool,
    pub description: String,
}

fn default_users() -> Vec<SeedUser> {
    vec![
        SeedUser {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            username: "admin".to_string(),
            email: "admin@example.com".to_string(),
        },
        SeedUser {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            username: "dev".to_string(),
            email: "dev@example.com".to_string(),
        },
    ]
}

fn default_flags() -> Vec<SeedFlag> {
    vec![
        SeedFlag {
            key: "new_dashboard".to_string(),
            enabled: false,
            description: "Enable the redesigned dashboard UI".to_string(),
        },
        SeedFlag {
            key: "beta_api".to_string(),
            enabled: true,
            description: "Expose beta API endpoints".to_string(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Individual seed steps
// ---------------------------------------------------------------------------

/// Ensure the `users` table exists and insert seed users (on-conflict do nothing).
///
/// # Errors
/// Returns [`SeedError::Database`] if the query fails.
pub async fn seed_users(pool: &PgPool) -> Result<usize, SeedError> {
    // Create table if it doesn't exist (dev convenience – migrations own this in prod).
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id          UUID PRIMARY KEY,
            username    TEXT NOT NULL UNIQUE,
            email       TEXT NOT NULL UNIQUE,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    let users = default_users();
    let mut inserted = 0usize;

    for user in &users {
        let rows = sqlx::query(
            r#"
            INSERT INTO users (id, username, email, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(user.id)
        .bind(&user.username)
        .bind(&user.email)
        .bind(Utc::now())
        .execute(pool)
        .await?
        .rows_affected();

        if rows > 0 {
            inserted += 1;
            info!(username = %user.username, "Seeded user");
        } else {
            warn!(username = %user.username, "User already exists – skipping");
        }
    }

    info!(count = inserted, "seed_users complete");
    Ok(inserted)
}

/// Ensure the `feature_flags` table exists and insert seed flags (on-conflict do nothing).
///
/// # Errors
/// Returns [`SeedError::Database`] if the query fails.
pub async fn seed_feature_flags(pool: &PgPool) -> Result<usize, SeedError> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS feature_flags (
            key         TEXT PRIMARY KEY,
            enabled     BOOLEAN NOT NULL DEFAULT FALSE,
            description TEXT NOT NULL DEFAULT '',
            updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    let flags = default_flags();
    let mut inserted = 0usize;

    for flag in &flags {
        let rows = sqlx::query(
            r#"
            INSERT INTO feature_flags (key, enabled, description, updated_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (key) DO NOTHING
            "#,
        )
        .bind(&flag.key)
        .bind(flag.enabled)
        .bind(&flag.description)
        .bind(Utc::now())
        .execute(pool)
        .await?
        .rows_affected();

        if rows > 0 {
            inserted += 1;
            info!(key = %flag.key, enabled = flag.enabled, "Seeded feature flag");
        } else {
            warn!(key = %flag.key, "Feature flag already exists – skipping");
        }
    }

    info!(count = inserted, "seed_feature_flags complete");
    Ok(inserted)
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Run all seed steps in order.
///
/// Each step is idempotent. A failure in one step does **not** abort the
/// remaining steps – all errors are collected and returned together.
///
/// # Errors
/// Returns the first [`SeedError`] encountered, if any.
pub async fn run_all(pool: &PgPool) -> Result<(), SeedError> {
    info!("Starting database seed");
    // Run each step and collect errors so one failing step doesn't abort the rest.
    let mut errors: Vec<SeedError> = Vec::new();

    if let Err(e) = seed_users(pool).await {
        errors.push(SeedError::StepFailed {
            step: "seed_users".to_string(),
            reason: e.to_string(),
        });
    }

    if let Err(e) = seed_feature_flags(pool).await {
        errors.push(SeedError::StepFailed {
            step: "seed_feature_flags".to_string(),
            reason: e.to_string(),
        });
    }

    if errors.is_empty() {
        info!("Database seed complete");
        Ok(())
    } else if errors.len() == 1 {
        Err(errors.remove(0))
    } else {
        Err(SeedError::Multiple(errors))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests that do not require a live database.

    #[test]
    fn test_default_users_are_valid() {
        let users = default_users();
        assert!(!users.is_empty());
        for u in &users {
            assert!(!u.username.is_empty());
            assert!(u.email.contains('@'));
        }
    }

    #[test]
    fn test_default_flags_are_valid() {
        let flags = default_flags();
        assert!(!flags.is_empty());
        for f in &flags {
            assert!(!f.key.is_empty());
            assert!(!f.description.is_empty());
        }
    }

    #[test]
    fn test_seed_error_display() {
        let err = SeedError::StepFailed {
            step: "seed_users".to_string(),
            reason: "connection refused".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("seed_users"));
        assert!(msg.contains("connection refused"));
    }

    #[test]
    fn test_default_user_ids_are_deterministic() {
        let a = default_users();
        let b = default_users();
        for (u1, u2) in a.iter().zip(b.iter()) {
            assert_eq!(u1.id, u2.id);
        }
    }

    #[test]
    fn test_no_duplicate_user_ids() {
        let users = default_users();
        let mut ids: Vec<Uuid> = users.iter().map(|u| u.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), users.len());
    }

    #[test]
    fn test_no_duplicate_flag_keys() {
        let flags = default_flags();
        let mut keys: Vec<&str> = flags.iter().map(|f| f.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), flags.len());
    }
}
