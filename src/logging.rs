//! Logging initialization via tracing-subscriber.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize structured logging.
///
/// `level` should be one of: trace, debug, info, warn, error.
/// `output` should be "stdout" (default) or "json".
pub fn init(level: &str, output: &str) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let fmt_layer = match output {
        "json" => fmt::layer().json().with_target(true).boxed(),
        _ => fmt::layer().with_target(true).boxed(),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    tracing::info!("Logging initialized (level={}, output={})", level, output);
}
