//! User factory for creating test users.
//!
//! Provides builders and factory functions for creating User domain objects
//! with customizable attributes for testing purposes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// User role in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Role {
    /// Regular user with basic permissions.
    #[default]
    User,
    /// Moderator with content management permissions.
    Moderator,
    /// Administrator with full system access.
    Admin,
}

/// User domain model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique identifier for the user.
    pub id: Uuid,
    /// User's email address (unique).
    pub email: String,
    /// User's display name.
    pub username: String,
    /// Hashed password.
    pub password_hash: String,
    /// User's role in the system.
    pub role: Role,
    /// Whether the user account is active.
    pub is_active: bool,
    /// Timestamp when the user was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp of the last login.
    pub last_login_at: Option<DateTime<Utc>>,
    /// Number of failed login attempts.
    pub failed_login_attempts: u32,
}

impl Default for User {
    fn default() -> Self {
        Self::new()
    }
}

impl User {
    /// Creates a new user with default values.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            email: format!("user{}@example.com", Uuid::new_v4()),
            username: format!("user_{}", &Uuid::new_v4().to_string()[..8]),
            password_hash: "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5GyYqKx8p/v2S"
                .to_string(),
            role: Role::default(),
            is_active: true,
            created_at: Utc::now(),
            last_login_at: None,
            failed_login_attempts: 0,
        }
    }

    /// Creates a new user with a specific email.
    pub fn with_email(email: impl Into<String>) -> Self {
        let mut user = Self::new();
        user.email = email.into();
        user
    }

    /// Creates a new admin user.
    pub fn admin() -> Self {
        let mut user = Self::new();
        user.role = Role::Admin;
        user
    }

    /// Creates an inactive user.
    pub fn inactive() -> Self {
        let mut user = Self::new();
        user.is_active = false;
        user
    }
}

/// Builder for creating User instances with custom attributes.
#[derive(Debug, Clone)]
pub struct UserFactory {
    user: User,
}

impl Default for UserFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl UserFactory {
    /// Creates a new UserFactory with default values.
    pub fn new() -> Self {
        Self { user: User::new() }
    }

    /// Creates a UserFactory from an existing User.
    pub fn from_user(user: User) -> Self {
        Self { user }
    }

    /// Sets the user's ID.
    pub fn id(mut self, id: Uuid) -> Self {
        self.user.id = id;
        self
    }

    /// Sets the user's email.
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.user.email = email.into();
        self
    }

    /// Sets the user's username.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.user.username = username.into();
        self
    }

    /// Sets the user's password hash.
    pub fn password_hash(mut self, password_hash: impl Into<String>) -> Self {
        self.user.password_hash = password_hash.into();
        self
    }

    /// Sets the user's role.
    pub fn role(mut self, role: Role) -> Self {
        self.user.role = role;
        self
    }

    /// Sets the user as admin.
    pub fn is_admin(mut self, is_admin: bool) -> Self {
        self.user.role = if is_admin { Role::Admin } else { Role::User };
        self
    }

    /// Sets the user's active status.
    pub fn is_active(mut self, is_active: bool) -> Self {
        self.user.is_active = is_active;
        self
    }

    /// Sets the user's created_at timestamp.
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.user.created_at = created_at;
        self
    }

    /// Sets the user's last_login_at timestamp.
    pub fn last_login_at(mut self, last_login_at: Option<DateTime<Utc>>) -> Self {
        self.user.last_login_at = last_login_at;
        self
    }

    /// Sets the number of failed login attempts.
    pub fn failed_login_attempts(mut self, attempts: u32) -> Self {
        self.user.failed_login_attempts = attempts;
        self
    }

    /// Builds and returns the User instance.
    pub fn finish(self) -> User {
        self.user
    }

    /// Builds and returns a boxed User instance.
    pub fn finish_boxed(self) -> Box<User> {
        Box::new(self.user)
    }
}

/// Creates a User with default values.
///
/// # Example
///
/// ```
/// use backend::test_utils::factories::user::create;
///
/// let user = create();
/// assert!(user.is_active);
/// ```
pub fn create() -> User {
    User::new()
}

/// Returns a new UserFactory builder.
pub fn build() -> UserFactory {
    UserFactory::new()
}

/// Creates a User with customizations applied via a closure.
///
/// # Example
///
/// ```
/// use backend::test_utils::factories::user::create_with;
///
/// let user = create_with(|u| {
///     u.email = "admin@example.com".to_string();
///     u.role = backend::test_utils::factories::user::Role::Admin;
/// });
/// assert_eq!(user.email, "admin@example.com");
/// ```
pub fn create_with<F>(f: F) -> User
where
    F: FnOnce(&mut User),
{
    let mut user = User::new();
    f(&mut user);
    user
}

/// Creates multiple Users with default values.
///
/// # Example
///
/// ```
/// use backend::test_utils::factories::user::create_many;
///
/// let users = create_many(5);
/// assert_eq!(users.len(), 5);
/// ```
pub fn create_many(count: usize) -> Vec<User> {
    (0..count).map(|_| User::new()).collect()
}

/// Creates multiple Users with a builder function applied to each.
///
/// # Example
///
/// ```
/// use backend::test_utils::factories::user::create_many_with;
///
/// let users = create_many_with(3, |u, i| {
///     u.email = format!("user{}@example.com", i);
/// });
/// assert_eq!(users.len(), 3);
/// assert_eq!(users[0].email, "user0@example.com");
/// ```
pub fn create_many_with<F>(count: usize, f: F) -> Vec<User>
where
    F: Fn(&mut User, usize),
{
    (0..count)
        .map(|i| {
            let mut user = User::new();
            f(&mut user, i);
            user
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_user_with_defaults() {
        let user = create();
        assert!(!user.id.is_nil());
        assert!(user.email.contains("@example.com"));
        assert!(user.is_active);
        assert_eq!(user.role, Role::User);
    }

    #[test]
    fn test_build_user_helper() {
        let user = build().email("helper@test.com").is_admin(true).finish();

        assert_eq!(user.email, "helper@test.com");
        assert_eq!(user.role, Role::Admin);
    }

    #[test]
    fn test_create_user_with_email() {
        let user = User::with_email("test@example.com");
        assert_eq!(user.email, "test@example.com");
    }

    #[test]
    fn test_create_admin_user() {
        let user = User::admin();
        assert_eq!(user.role, Role::Admin);
    }

    #[test]
    fn test_create_inactive_user() {
        let user = User::inactive();
        assert!(!user.is_active);
    }

    #[test]
    fn test_user_factory_basic() {
        let user = UserFactory::new()
            .email("factory@example.com")
            .is_admin(true)
            .finish();

        assert_eq!(user.email, "factory@example.com");
        assert_eq!(user.role, Role::Admin);
    }

    #[test]
    fn test_user_factory_all_options() {
        let id = Uuid::new_v4();
        let created_at = Utc::now();

        let user = UserFactory::new()
            .id(id)
            .email("full@example.com")
            .username("fulluser")
            .password_hash("hash123")
            .role(Role::Moderator)
            .is_active(false)
            .created_at(created_at)
            .last_login_at(Some(Utc::now()))
            .failed_login_attempts(3)
            .finish();

        assert_eq!(user.id, id);
        assert_eq!(user.email, "full@example.com");
        assert_eq!(user.username, "fulluser");
        assert_eq!(user.password_hash, "hash123");
        assert_eq!(user.role, Role::Moderator);
        assert!(!user.is_active);
        assert_eq!(user.created_at, created_at);
        assert!(user.last_login_at.is_some());
        assert_eq!(user.failed_login_attempts, 3);
    }

    #[test]
    fn test_create_with_closure() {
        let user = create_with(|u| {
            u.email = "closure@example.com".to_string();
            u.is_active = false;
        });
        assert_eq!(user.email, "closure@example.com");
        assert!(!user.is_active);
    }

    #[test]
    fn test_create_many() {
        let users = create_many(5);
        assert_eq!(users.len(), 5);

        // All users should have unique IDs
        let ids: Vec<_> = users.iter().map(|u| u.id).collect();
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn test_create_many_with() {
        let users = create_many_with(3, |u, i| {
            u.email = format!("user{}@test.com", i);
        });

        assert_eq!(users.len(), 3);
        assert_eq!(users[0].email, "user0@test.com");
        assert_eq!(users[1].email, "user1@test.com");
        assert_eq!(users[2].email, "user2@test.com");
    }

    #[test]
    fn test_user_factory_from_user() {
        let original = create();
        let factory = UserFactory::from_user(original.clone());
        let user = factory.email("modified@example.com").finish();

        assert_eq!(user.id, original.id);
        assert_eq!(user.email, "modified@example.com");
    }

    #[test]
    fn test_user_serialization() {
        let user = create();
        let json = serde_json::to_string(&user).unwrap();
        let parsed: User = serde_json::from_str(&json).unwrap();

        assert_eq!(user.id, parsed.id);
        assert_eq!(user.email, parsed.email);
    }
}
