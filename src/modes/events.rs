//! Single-project events polling mode.

use crate::config::Config;
use crate::db::{DeploymentDb, DeploymentRecord};
use crate::deploy::Deployer;
use crate::filter::Filter;
use crate::gitlab::GitLabClient;
use crate::notify::Notifier;
use crate::types::PushEvent;
use std::sync::Arc;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub async fn watch_events(cfg: &Config) -> anyhow::Result<()> {
    let project = cfg.deploy.project.as_deref().unwrap_or("");
    if project.is_empty() {
        anyhow::bail!("Events mode requires --project (or deploy.project in config)");
    }

    let filter = Filter::load(cfg.filter.file.as_deref())?;

    let client = Arc::new(GitLabClient::new(cfg.gitlab.url.clone(), cfg.gitlab.token.clone())?);
    let deployer = Arc::new(Deployer::new(
        cfg.deploy.src_dir.clone(),
        cfg.deploy.scripts_dir.clone(),
        cfg.deploy.script_timeout_secs,
        cfg.harbor.password.clone(),
    ));
    let notifier = Arc::new(Notifier::new(cfg.notify.msg_platform_url.clone())?);
    let db = Arc::new(DeploymentDb::open(&cfg.database.path).await?);

    let encoded = project.replace('/', "%2F");
    let mut last_event_id: Option<u64> = None;
    let mut poll_count: u64 = 0;
    let mut active_token: Option<CancellationToken> = None;

    info!("Events mode: monitoring {} (interval {}s)", project, cfg.gitlab.poll_interval_secs);

    let poll_interval = std::time::Duration::from_secs(cfg.gitlab.poll_interval_secs);

    loop {
        if signal::ctrl_c().await.is_ok() {
            info!("SIGINT received, shutting down...");
            break;
        }

        match client.get_project_events(&encoded, 5).await {
            Ok(events) => {
                if let Some(latest) = events.first() {
                    let event_id = latest.id;
                    if last_event_id.is_none() {
                        last_event_id = Some(event_id);
                        info!("Initial event ID: {}", event_id);
                    } else if event_id != last_event_id.unwrap() {
                        last_event_id = Some(event_id);

                        let branch = latest.push_data.as_ref()
                            .map(|pd| pd.r#ref.as_str())
                            .unwrap_or("");

                        if !filter.matches(project, branch) {
                            continue;
                        }

                        let push_event = match PushEvent::from_event(latest, Some(project)) {
                            Some(pe) => pe,
                            None => {
                                warn!("Failed to parse push event");
                                continue;
                            }
                        };

                        // Cancel previous deployment
                        if let Some(token) = active_token.take() {
                            info!("Cancelling previous deployment for {}", project);
                            token.cancel();
                        }

                        let cancel = CancellationToken::new();
                        let cancel_clone = cancel.clone();
                        let deployer = deployer.clone();
                        let client = client.clone();
                        let notifier = notifier.clone();
                        let db = db.clone();
                        let notify_author = cfg.notify.notify_author;

                        active_token = Some(cancel);

                        let gitlab_url = cfg.gitlab.url.clone();
                        let gitlab_token = cfg.gitlab.token.clone();

                        tokio::spawn(async move {
                            let result = deployer
                                .deploy(&push_event, &gitlab_url, &gitlab_token, cancel_clone)
                                .await;

                            if result.cancelled {
                                return;
                            }

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

                            if notify_author {
                                let email = client.get_user_email(push_event.author_id).await.unwrap_or_default();
                                notifier.notify(&push_event, &result, &email).await;
                            }
                        });
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

    info!("ru_deployer events-mode shut down");
    Ok(())
}
