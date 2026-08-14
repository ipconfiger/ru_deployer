mod config;
mod db;
mod deploy;
mod filter;
mod git;
mod gitlab;
mod logging;
mod modes;
mod notify;
mod oneshot;
mod types;

use clap::{Parser, Subcommand};
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

    /// Branch for commits mode (or override branch in --once mode)
    #[arg(long)]
    branch: Option<String>,

    /// Run a single deployment once and exit (no polling loop)
    #[arg(long)]
    once: bool,

    /// Service name for --once mode (e.g. "api", "horizon", "flint")
    #[arg(long)]
    service: Option<String>,

    /// Query sub-commands (history/stats) — mutually exclusive with polling
    #[command(subcommand)]
    command: Option<Command>,
}

/// CLI sub-commands that query the deployment database and exit
/// (T2: tech debt plan — deployment history inspection).
#[derive(Subcommand, Debug)]
enum Command {
    /// Show deployment history for a project
    History {
        /// Project path (e.g. "dev-team/api")
        project: String,
        /// Filter by branch
        #[arg(long)]
        branch: Option<String>,
        /// Number of records to show (default 10)
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Show deployment statistics for a project
    Stats {
        /// Project path (e.g. "dev-team/api")
        project: String,
        /// Look-back window in days (default 30)
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
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
    let cli_branch = args.branch.clone();
    if let Some(b) = args.branch {
        cfg.deploy.branch = Some(b);
    }

    // Sub-commands (history/stats): query the DB and exit without polling.
    // Deliberately before logging::init so plain table output is not mixed
    // with tracing logs on stdout.
    if let Some(cmd) = args.command {
        return run_command(&cfg, cmd).await;
    }

    // Initialize logging
    logging::init(&cfg.logging.level, &cfg.logging.output);

    tracing::info!(
        "ru_deployer starting (mode={}, gitlab={})",
        cfg.deploy.mode,
        cfg.gitlab.url
    );

    // One-shot mode: deploy a single service once and exit (no polling loop)
    if args.once {
        let service = args.service.as_deref().ok_or_else(|| {
            anyhow::anyhow!("--once requires --service <name> (e.g. api, horizon, flint)")
        })?;
        oneshot::run(&cfg, service, cli_branch.as_deref()).await?;
        return Ok(());
    }

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

/// Execute a query sub-command (history/stats): open the deployment DB,
/// query, print results, and exit. No polling, no notifier, no deployer.
async fn run_command(cfg: &config::Config, cmd: Command) -> anyhow::Result<()> {
    let db = db::DeploymentDb::open(&cfg.database.path).await?;
    match cmd {
        Command::History {
            project,
            branch,
            limit,
        } => {
            let records = if let Some(b) = branch {
                db.recent_by_branch(&project, &b, limit).await?
            } else {
                db.recent(&project, limit).await?
            };
            println!("{}", format_history(&records));
        }
        Command::Stats { project, days } => {
            let stats = db.stats(&project, days).await?;
            println!("project: {}", project);
            println!("days:    {}", days);
            println!("total:   {}", stats.total);
            println!("success: {}", stats.success);
            println!("failed:  {}", stats.failed);
        }
    }
    Ok(())
}

/// Format deployment history as a plain-text table.
fn format_history(records: &[db::DeploymentRecord]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<6} {:<20} {:<8} {:<20} {:<10} {:<6} {:<8} {:<8}\n",
        "id", "time", "type", "branch", "commit", "exit", "status", "dur(s)"
    ));
    for r in records {
        let commit: String = r.commit_sha.chars().take(8).collect();
        out.push_str(&format!(
            "{:<6} {:<20} {:<8} {:<20} {:<10} {:<6} {:<8} {:<8}\n",
            r.id,
            r.created_at,
            r.event_type,
            r.branch,
            commit,
            r.exit_code,
            r.status,
            r.duration_ms / 1000,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_polling_mode() {
        let args = Args::try_parse_from(["ru_deployer", "--mode", "multi"]).unwrap();
        assert!(args.command.is_none());
        assert_eq!(args.mode, "multi");
    }

    #[test]
    fn test_parse_history() {
        let args =
            Args::try_parse_from(["ru_deployer", "history", "dev-team/api", "--limit", "5"])
                .unwrap();
        match args.command {
            Some(Command::History {
                project,
                branch,
                limit,
            }) => {
                assert_eq!(project, "dev-team/api");
                assert!(branch.is_none());
                assert_eq!(limit, 5);
            }
            _ => panic!("expected history sub-command"),
        }
    }

    #[test]
    fn test_parse_history_with_branch() {
        let args =
            Args::try_parse_from(["ru_deployer", "history", "dev-team/api", "--branch", "main"])
                .unwrap();
        match args.command {
            Some(Command::History { branch, .. }) => {
                assert_eq!(branch.as_deref(), Some("main"));
            }
            _ => panic!("expected history sub-command"),
        }
    }

    #[test]
    fn test_parse_stats() {
        let args = Args::try_parse_from(["ru_deployer", "stats", "dev-team/api", "--days", "7"])
            .unwrap();
        match args.command {
            Some(Command::Stats { project, days }) => {
                assert_eq!(project, "dev-team/api");
                assert_eq!(days, 7);
            }
            _ => panic!("expected stats sub-command"),
        }
    }

    #[test]
    fn test_format_history() {
        let records = vec![db::DeploymentRecord {
            id: 42,
            created_at: "2026-08-13 10:00:00".into(),
            project: "dev-team/api".into(),
            branch: "main".into(),
            commit_sha: "abcdef1234567890abcdef".into(),
            author_name: "u".into(),
            author_email: "".into(),
            event_id: 1,
            event_type: "push".into(),
            exit_code: 0,
            status: "success".into(),
            stdout_tail: "".into(),
            stderr_tail: "".into(),
            duration_ms: 120000,
        }];
        let out = format_history(&records);
        assert!(out.contains("42"));
        assert!(out.contains("abcdef12"));
        assert!(out.contains("push"));
        assert!(out.contains("success"));
        assert!(out.contains("120"), "duration should be shown in seconds");
        assert!(out.starts_with("id "), "table header expected");
    }
}
