//! Commits-based monitoring mode — monitor-only, no deployment.
//!
//! Polls the /repository/commits API and prints commit changes.
//! Aligned with Python listen_push.py behavior: commits mode does NOT trigger deployment.

use crate::config::Config;
use crate::gitlab::GitLabClient;
use tokio::signal;
use tracing::{error, info};

pub async fn watch_commits(cfg: &Config) -> anyhow::Result<()> {
    let project = cfg.deploy.project.as_deref().unwrap_or("");
    if project.is_empty() {
        anyhow::bail!("Commits mode requires --project (or deploy.project in config)");
    }
    let branch = cfg.deploy.branch.as_deref().unwrap_or("main");

    let client = GitLabClient::new(cfg.gitlab.url.clone(), cfg.gitlab.token.clone())?;
    let encoded = project.replace('/', "%2F");
    let mut old_commit: Option<String> = None;
    let mut poll_count: u64 = 0;

    info!(
        "Commits mode (monitor-only): {} @ {} (interval {}s)",
        project, branch, cfg.gitlab.poll_interval_secs
    );

    let poll_interval = std::time::Duration::from_secs(cfg.gitlab.poll_interval_secs);

    loop {
        if signal::ctrl_c().await.is_ok() {
            info!("SIGINT received, shutting down...");
            break;
        }

        match client.get_commits(&encoded, branch, 1).await {
            Ok(commits) => {
                if let Some(c) = commits.first() {
                    let new_commit = c.id.clone();
                    if old_commit.is_none() {
                        old_commit = Some(new_commit.clone());
                        info!("Initial commit: {:.8} ({})", c.id, c.title);
                    } else if new_commit != *old_commit.as_ref().unwrap() {
                        info!("New commit detected:");
                        info!("  SHA:        {}", c.id);
                        info!("  Title:      {}", c.title);
                        info!("  Author:     {}", c.author_name);
                        if let Some(t) = c.created_at {
                            info!("  Time:       {}", t);
                        }
                        info!("  URL:        {}", c.web_url);
                        old_commit = Some(new_commit);
                    }
                }
            }
            Err(e) => error!("API request failed: {}", e),
        }

        poll_count += 1;
        if poll_count % 30 == 0 {
            info!(
                "Heartbeat - {} polls [commit={:.8}]",
                poll_count,
                old_commit.as_deref().unwrap_or("?")
            );
        }

        tokio::time::sleep(poll_interval).await;
    }

    info!("ru_deployer commits-mode shut down");
    Ok(())
}
