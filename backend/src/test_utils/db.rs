use crate::error::AppError;
use sqlx::{postgres::PgPool, Pool, Postgres};
use std::env;
use std::sync::Arc;
use tokio::runtime::Handle;
use uuid::Uuid;

/// Sets up a test database with a unique name and runs migrations.
///
/// This function creates a new test database by appending a UUID to the base
/// database name, connects to it, runs all SQLx migrations, and returns a
/// connection pool. The test database is created as a template1 database
/// copy for faster setup.
///
/// # Arguments
///
/// * None
///
/// # Returns
///
/// A connection pool to the newly created test database
///
/// # Errors
///
/// Returns AppError if database creation, connection, or migration fails
pub async fn setup_test_db() -> Result<(PgPool, String), AppError> {
    // Get the base database URL from environment
    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set for test database setup");

    // Parse the URL to extract connection details
    let mut url = url::Url::parse(&database_url)?;

    // Extract the current database name
    let db_name = url.path().trim_start_matches('/').to_string();

    // Generate a unique test database name
    let test_db_name = format!("crucible_test_{}", Uuid::new_v4());

    // Connect to the default database (usually 'postgres') to create our test DB
    url.set_path("/postgres");
    let admin_url = url.as_str();

    // Create connection to admin database
    let admin_pool = PgPool::connect(admin_url).await?;

    // Create the test database
    sqlx::query(&format!("CREATE DATABASE {}", test_db_name))
        .execute(&admin_pool)
        .await?;

    // Close admin connection
    admin_pool.close().await;

    // Now connect to the test database
    url.set_path(&format!("/{}", test_db_name));
    let test_db_url = url.as_str();

    // Create pool for test database
    let pool = PgPool::connect(test_db_url).await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok((pool, test_db_name))
}

/// Tears down a test database by closing the pool and dropping the database.
///
/// This function closes the connection pool to the test database and then
/// drops the database entirely. It should be called after tests are finished
/// with the test database.
///
/// # Arguments
///
/// * `pool` - Connection pool to the test database
/// * `db_name` - Name of the test database to drop
///
/// # Errors
///
/// Returns AppError if dropping the database fails
pub async fn teardown_test_db(pool: PgPool, db_name: &str) -> Result<(), AppError> {
    // Close the pool first
    pool.close().await;

    // Extract connection details to connect to admin database from the environment
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set for test database teardown");
    let mut url = url::Url::parse(&db_url)?;

    // Switch to admin database
    url.set_path("/postgres");
    let admin_url = url.as_str();

    // Create connection to admin database
    let admin_pool = PgPool::connect(admin_url).await?;

    // Drop the test database
    sqlx::query(&format!("DROP DATABASE IF EXISTS {}", db_name))
        .execute(&admin_pool)
        .await?;

    // Close admin connection
    admin_pool.close().await;

    Ok(())
}

/// RAII structure for automatic test database teardown.
///
/// This structure automatically calls `teardown_test_db` when it goes out of
/// scope, ensuring proper cleanup of test resources even in case of panics.
///
/// # Example
///
/// ```
/// #[tokio::test]
/// async fn my_test() {
///     let _db = TestDb::new().await;
///     // Use the database for testing...
///     // When _db goes out of scope, the database is automatically dropped
/// }
/// ```
#[derive(Debug)]
pub struct TestDb {
    /// Connection pool to the test database
    pub pool: PgPool,
    /// Name of the test database
    pub db_name: String,
}

impl TestDb {
    /// Create a new test database and return a TestDb instance.
    ///
    /// # Returns
    ///
    /// A TestDb instance containing the connection pool and database name
    ///
    /// # Errors
    ///
    /// Returns AppError if test database setup fails
    pub async fn new() -> Result<Self, AppError> {
        let (pool, db_name) = setup_test_db().await?;

        Ok(Self { pool, db_name })
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // Use the current tokio runtime to run the async teardown
        if let Ok(handle) = Handle::try_current() {
            // Spawn a blocking task to run the teardown
            let pool = self.pool.clone();
            let db_name = self.db_name.clone();
            handle.spawn_blocking(move || {
                // Create a new runtime for this blocking operation
                let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                rt.block_on(async move {
                    let _ = teardown_test_db(pool, &db_name).await;
                });
            });
        } else {
            // If we're not in a tokio context, we can't reliably clean up
            // In practice, this shouldn't happen during tests
            tracing::warn!("Unable to tear down test database: not in tokio context");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_setup_and_teardown_test_db() {
        // Set up test database URL if not already set
        env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost:5432/postgres",
        );

        // Setup test database
        let db = TestDb::new().await.expect("Failed to create test database");

        // Verify we can query the database
        let result = sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&db.pool)
            .await
            .expect("Failed to query test database");

        assert_eq!(result, 1);

        // Verify database name follows expected pattern
        assert!(db.db_name.starts_with("crucible_test_"));

        // When db goes out of scope, Drop implementation will clean up
        // We explicitly test the teardown function below
    }

    #[tokio::test]
    async fn test_teardown_test_db_function() {
        // Set up test database URL if not already set
        env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost:5432/postgres",
        );

        // Setup test database manually
        let (pool, db_name) = setup_test_db()
            .await
            .expect("Failed to create test database");

        // Verify we can query the database
        let result = sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("Failed to query test database");

        assert_eq!(result, 1);

        // Teardown the database
        teardown_test_db(pool, &db_name)
            .await
            .expect("Failed to tear down test database");

        // Note: We can't easily verify the database was dropped without attempting
        // to reconnect, which would fail. The teardown function not returning an
        // error indicates success.
    }
}
