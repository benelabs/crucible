//! Session factory for creating test sessions.
//!
//! Provides builders and factory functions for creating Session domain objects
//! with customizable attributes for testing purposes.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionStatus {
    /// Session is active.
    #[default]
    Active,
    /// Session has expired.
    Expired,
    /// Session was manually revoked.
    Revoked,
}

/// Type of session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SessionType {
    /// Regular user login session.
    #[default]
    Login,
    /// API token session.
    Api,
    /// Password reset session.
    PasswordReset,
    /// Email verification session.
    EmailVerification,
}

/// Session domain model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique identifier for the session.
    pub id: Uuid,
    /// User ID who owns the session.
    pub user_id: Uuid,
    /// Session type.
    pub session_type: SessionType,
    /// Status of the session.
    pub status: SessionStatus,
    /// IP address of the client.
    pub ip_address: Option<String>,
    /// User agent string.
    pub user_agent: Option<String>,
    /// Session token (hashed).
    pub token_hash: String,
    /// Timestamp when the session was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the session expires.
    pub expires_at: DateTime<Utc>,
    /// Timestamp when the session was last used.
    pub last_used_at: DateTime<Utc>,
    /// Number of times the session was used.
    pub use_count: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Creates a new session with default values.
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            session_type: SessionType::default(),
            status: SessionStatus::default(),
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: Some("Test Agent".to_string()),
            token_hash: "$2b$12$hash".to_string(),
            created_at: now,
            expires_at: now + Duration::days(7),
            last_used_at: now,
            use_count: 0,
        }
    }

    /// Creates a new session for a specific user.
    pub fn for_user(user_id: Uuid) -> Self {
        let mut session = Self::new();
        session.user_id = user_id;
        session
    }

    /// Creates an expired session.
    pub fn expired() -> Self {
        let mut session = Self::new();
        session.status = SessionStatus::Expired;
        session.expires_at = Utc::now() - Duration::days(1);
        session
    }

    /// Creates a revoked session.
    pub fn revoked() -> Self {
        let mut session = Self::new();
        session.status = SessionStatus::Revoked;
        session
    }

    /// Checks if the session is valid (active and not expired).
    pub fn is_valid(&self) -> bool {
        self.status == SessionStatus::Active && Utc::now() < self.expires_at
    }

    /// Records a session use.
    pub fn record_use(&mut self) {
        self.use_count += 1;
        self.last_used_at = Utc::now();
    }
}

/// Builder for creating Session instances.
#[derive(Debug, Clone)]
pub struct SessionFactory {
    session: Session,
}

impl Default for SessionFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionFactory {
    /// Creates a new SessionFactory with default values.
    pub fn new() -> Self {
        Self {
            session: Session::new(),
        }
    }

    /// Creates a SessionFactory from an existing Session.
    pub fn from_session(session: Session) -> Self {
        Self { session }
    }

    /// Sets the session's ID.
    pub fn id(mut self, id: Uuid) -> Self {
        self.session.id = id;
        self
    }

    /// Sets the user ID.
    pub fn user_id(mut self, user_id: Uuid) -> Self {
        self.session.user_id = user_id;
        self
    }

    /// Sets the session type.
    pub fn session_type(mut self, session_type: SessionType) -> Self {
        self.session.session_type = session_type;
        self
    }

    /// Sets the session status.
    pub fn status(mut self, status: SessionStatus) -> Self {
        self.session.status = status;
        self
    }

    /// Sets the IP address.
    pub fn ip_address(mut self, ip: Option<String>) -> Self {
        self.session.ip_address = ip;
        self
    }

    /// Sets the user agent.
    pub fn user_agent(mut self, user_agent: Option<String>) -> Self {
        self.session.user_agent = user_agent;
        self
    }

    /// Sets the token hash.
    pub fn token_hash(mut self, token_hash: impl Into<String>) -> Self {
        self.session.token_hash = token_hash.into();
        self
    }

    /// Sets the created_at timestamp.
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.session.created_at = created_at;
        self
    }

    /// Sets the expires_at timestamp.
    pub fn expires_at(mut self, expires_at: DateTime<Utc>) -> Self {
        self.session.expires_at = expires_at;
        self
    }

    /// Sets the session to expire in a given number of days.
    pub fn expires_in_days(mut self, days: i64) -> Self {
        self.session.expires_at = Utc::now() + Duration::days(days);
        self
    }

    /// Sets the session to expire in a given number of hours.
    pub fn expires_in_hours(mut self, hours: i64) -> Self {
        self.session.expires_at = Utc::now() + Duration::hours(hours);
        self
    }

    /// Sets the last_used_at timestamp.
    pub fn last_used_at(mut self, last_used_at: DateTime<Utc>) -> Self {
        self.session.last_used_at = last_used_at;
        self
    }

    /// Sets the use count.
    pub fn use_count(mut self, count: u32) -> Self {
        self.session.use_count = count;
        self
    }

    /// Builds and returns the Session instance.
    pub fn finish(self) -> Session {
        self.session
    }
}

/// Creates a Session with default values.
pub fn create() -> Session {
    Session::new()
}

/// Returns a new SessionFactory builder.
pub fn build() -> SessionFactory {
    SessionFactory::new()
}

/// Creates a Session for a specific user.
pub fn create_for_user(user_id: Uuid) -> Session {
    Session::for_user(user_id)
}

/// Creates a Session with customizations applied via a closure.
pub fn create_with<F>(f: F) -> Session
where
    F: FnOnce(&mut Session),
{
    let mut session = Session::new();
    f(&mut session);
    session
}

/// Creates multiple Sessions with default values.
pub fn create_many(count: usize) -> Vec<Session> {
    (0..count).map(|_| Session::new()).collect()
}

/// Creates multiple Sessions for a specific user.
pub fn create_many_for_user(user_id: Uuid, count: usize) -> Vec<Session> {
    (0..count).map(|_| Session::for_user(user_id)).collect()
}

/// Creates multiple Sessions with a builder function applied to each.
pub fn create_many_with<F>(count: usize, f: F) -> Vec<Session>
where
    F: Fn(&mut Session, usize),
{
    (0..count)
        .map(|i| {
            let mut session = Session::new();
            f(&mut session, i);
            session
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session_with_defaults() {
        let session = create();
        assert!(!session.id.is_nil());
        assert_eq!(session.status, SessionStatus::Active);
        assert_eq!(session.session_type, SessionType::Login);
        assert!(session.is_valid());
    }

    #[test]
    fn test_create_session_for_user() {
        let user_id = Uuid::new_v4();
        let session = create_for_user(user_id);
        assert_eq!(session.user_id, user_id);
    }

    #[test]
    fn test_create_expired_session() {
        let session = Session::expired();
        assert_eq!(session.status, SessionStatus::Expired);
        assert!(!session.is_valid());
    }

    #[test]
    fn test_create_revoked_session() {
        let session = Session::revoked();
        assert_eq!(session.status, SessionStatus::Revoked);
        assert!(!session.is_valid());
    }

    #[test]
    fn test_session_is_valid() {
        let session = create();
        assert!(session.is_valid());
    }

    #[test]
    fn test_session_record_use() {
        let mut session = create();
        assert_eq!(session.use_count, 0);

        session.record_use();
        assert_eq!(session.use_count, 1);
        assert!(session.last_used_at > session.created_at);
    }

    #[test]
    fn test_session_factory_basic() {
        let session = SessionFactory::new()
            .session_type(SessionType::Api)
            .expires_in_days(30)
            .finish();

        assert_eq!(session.session_type, SessionType::Api);
    }

    #[test]
    fn test_session_factory_all_options() {
        let user_id = Uuid::new_v4();
        let created_at = Utc::now();

        let session = SessionFactory::new()
            .id(Uuid::new_v4())
            .user_id(user_id)
            .session_type(SessionType::PasswordReset)
            .status(SessionStatus::Active)
            .ip_address(Some("192.168.1.1".to_string()))
            .user_agent(Some("Mozilla/5.0".to_string()))
            .token_hash("custom_hash")
            .created_at(created_at)
            .expires_at(created_at + Duration::hours(1))
            .last_used_at(created_at)
            .use_count(5)
            .finish();

        assert_eq!(session.user_id, user_id);
        assert_eq!(session.session_type, SessionType::PasswordReset);
        assert_eq!(session.ip_address, Some("192.168.1.1".to_string()));
        assert_eq!(session.token_hash, "custom_hash");
        assert_eq!(session.use_count, 5);
    }

    #[test]
    fn test_session_factory_expires_in_hours() {
        let session = SessionFactory::new().expires_in_hours(2).finish();

        let expected_expires = Utc::now() + Duration::hours(2);
        // Allow 1 second tolerance
        assert!((session.expires_at - expected_expires).num_seconds().abs() <= 1);
    }

    #[test]
    fn test_create_with_closure() {
        let session = create_with(|s| {
            s.status = SessionStatus::Revoked;
            s.ip_address = Some("10.0.0.1".to_string());
        });

        assert_eq!(session.status, SessionStatus::Revoked);
        assert_eq!(session.ip_address, Some("10.0.0.1".to_string()));
    }

    #[test]
    fn test_create_many() {
        let sessions = create_many(5);
        assert_eq!(sessions.len(), 5);
    }

    #[test]
    fn test_create_many_for_user() {
        let user_id = Uuid::new_v4();
        let sessions = create_many_for_user(user_id, 3);

        assert_eq!(sessions.len(), 3);
        for session in sessions {
            assert_eq!(session.user_id, user_id);
        }
    }

    #[test]
    fn test_session_serialization() {
        let session = create();
        let json = serde_json::to_string(&session).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(session.id, parsed.id);
        assert_eq!(session.user_id, parsed.user_id);
    }
}
