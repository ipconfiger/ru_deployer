mod config;
mod db;
mod deploy;
mod filter;
mod git;
mod gitlab;
mod logging;
mod modes;
mod notify;
mod types;

use clap::Parser;
use std::path::PathBuf;

/// GitLab push event watcher and automated deployer.
#[derive(Parser, Debug)]
#[command(name = "ru_deployer", version, about)]
struct Args {
    /// Polling mode: multi (default), events, global, commits
    #[arg(long, default_value = "multi")]
    mode: String,

    /// Path to config file
    #[arg(long, default_value = "./config.toml")]
    config: PathBuf,

    /// Path to filter file (overrides config value)
    #[arg(long)]
    filter: Option<PathBuf>,

    /// Project path for events/commits mode (e.g. "dev-team/api")
    #[arg(long)]
    project: Option<String>,

    /// Branch for commits mode
    #[arg(long)]
    branch: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env before config parsing (so HARBOR_PASSWORD etc. are available)
    let _ = dotenvy::dotenv();

    let args = Args::parse();

    // Load configuration
    let mut cfg = config::load(&args.config)?;

    // CLI overrides
    cfg.deploy.mode = args.mode.clone();
    if let Some(f) = args.filter {
        cfg.filter.file = Some(f);
    }
    if let Some(p) = args.project {
        cfg.deploy.project = Some(p);
    }
    if let Some(b) = args.branch {
        cfg.deploy.branch = Some(b);
    }

    // Initialize logging
    logging::init(&cfg.logging.level, &cfg.logging.output);

    tracing::info!(
        "ru_deployer starting (mode={}, gitlab={})",
        cfg.deploy.mode,
        cfg.gitlab.url
    );

    match cfg.deploy.mode.as_str() {
        "multi" => {
            modes::watch_multi(&cfg).await?;
        }
        "events" => {
            modes::events::watch_events(&cfg).await?;
        }
        "global" => {
            modes::global::watch_global(&cfg).await?;
        }
        "commits" => {
            modes::commits::watch_commits(&cfg).await?;
        }
        _ => {
            anyhow::bail!("Unknown mode: {} (valid: multi, events, global, commits)", cfg.deploy.mode);
        }
    }

    Ok(())
}
