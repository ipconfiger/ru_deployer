//! Global events polling mode — monitors all visible projects.

use crate::config::Config;
use crate::db::{DeploymentDb, DeploymentRecord};
use crate::deploy::{DeployResult, Deployer};
use crate::filter::Filter;
use crate::gitlab::GitLabClient;
use crate::notify::Notifier;
use crate::types::PushEvent;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

pub async fn watch_global(cfg: &Config) -> anyhow::Result<()> {
    let filter = Filter::load(cfg.filter.file.as_deref())?;
    let filter_info = if !filter.is_empty() {
        format!(" (filter: {} projects)", filter.project_paths().count())
    } else {
        String::new()
    };

    let client = Arc::new(GitLabClient::new(cfg.gitlab.url.clone(), cfg.gitlab.token.clone())?);
    let deployer = Arc::new(Deployer::new(
        cfg.deploy.src_dir.clone(),
        cfg.deploy.scripts_dir.clone(),
        cfg.deploy.script_timeout_secs,
        cfg.harbor.password.clone(),
    ));
    let notifier = Arc::new(Notifier::new(cfg.notify.msg_platform_url.clone())?);
    let db = Arc::new(DeploymentDb::open(&cfg.database.path).await?);

    let mut last_event_id: Option<u64> = None;
    let mut poll_count: u64 = 0;

    info!("Global mode: monitoring all push events{} (interval {}s)",
        filter_info, cfg.gitlab.poll_interval_secs);

    let poll_interval = std::time::Duration::from_secs(cfg.gitlab.poll_interval_secs);

    loop {
        if signal::ctrl_c().await.is_ok() {
            info!("SIGINT received, shutting down...");
            break;
        }

        match client.get_global_events(10).await {
            Ok(events) => {
                if let Some(latest) = events.first() {
                    let event_id = latest.id;
                    if last_event_id.is_none() {
                        last_event_id = Some(event_id);
                        info!("Initial event ID: {}", event_id);
                    } else if event_id != last_event_id.unwrap() {
                        last_event_id = Some(event_id);

                        let project_path = match client.lookup_project(latest.project_id).await {
                            Ok(Some(p)) => p,
                            _ => {
                                warn!("Could not lookup project ID {}", latest.project_id);
                                continue;
                            }
                        };

                        let branch = latest.push_data.as_ref()
                            .map(|pd| pd.r#ref.as_str())
                            .unwrap_or("");

                        if !filter.matches(&project_path, branch) {
                            continue;
                        }

                        let push_event = match PushEvent::from_event(latest, Some(&project_path)) {
                            Some(pe) => pe,
                            None => {
                                warn!("Failed to parse push event for {}", project_path);
                                continue;
                            }
                        };

                        info!("Push detected: {} @ {} (commit: {:.8})",
                            push_event.project, push_event.branch, push_event.commit);

                        let result = deployer
                            .deploy(&push_event, &cfg.gitlab.url, &cfg.gitlab.token)
                            .await
                            .unwrap_or_else(|e| DeployResult {
                                exit_code: -1, stdout: String::new(),
                                stderr: format!("{}", e), commit: String::new(),
                                duration: std::time::Duration::ZERO,
                            });

                        let record = DeploymentRecord {
                            project: push_event.project.clone(),
                            branch: push_event.branch.clone(),
                            commit_sha: push_event.commit.clone(),
                            author_name: push_event.author_name.clone(),
                            author_email: String::new(),
                            event_id: push_event.event_id,
                            exit_code: result.exit_code,
                            status: if result.exit_code == 0 { "success".into() } else { "failed".into() },
                            stdout_tail: result.stdout.clone(),
                            stderr_tail: result.stderr.clone(),
                            duration_ms: result.duration.as_millis() as i64,
                        };

                        let _ = db.insert(&record).await;

                        if cfg.notify.notify_author {
                            let email = client.get_user_email(push_event.author_id).await.unwrap_or_default();
                            notifier.notify(&push_event, &result, &email).await;
                        }
                    }
                }
            }
            Err(e) => error!("API request failed: {}", e),
        }

        poll_count += 1;
        if poll_count % 30 == 0 {
            info!("Heartbeat - {} polls [event_id={}]", poll_count,
                last_event_id.map_or("?".into(), |id| id.to_string()));
        }

        tokio::time::sleep(poll_interval).await;
    }

    info!("ru_deployer global-mode shut down");
    Ok(())
}
