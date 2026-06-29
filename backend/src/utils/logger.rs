use crate::config::Environment;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes structured logging for the backend.
///
/// Development uses a pretty formatter for readability, while staging and
/// production emit JSON logs for easier ingestion by log pipelines.
pub fn init_tracing(log_level: &str, env: Environment) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let subscriber = tracing_subscriber::registry().with(filter);

    match env {
        Environment::Development => {
            let fmt_layer = tracing_subscriber::fmt::layer()
                .pretty()
                .with_thread_ids(true)
                .with_target(true);

            let _ = subscriber.with(fmt_layer).try_init();
        }
        Environment::Staging | Environment::Production => {
            let fmt_layer = tracing_subscriber::fmt::layer()
                .json()
                .with_span_list(true)
                .with_current_span(true);

            let _ = subscriber.with(fmt_layer).try_init();
        }
    }
}
