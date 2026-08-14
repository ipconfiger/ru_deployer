//! Single-project events polling mode.

use crate::config::Config;
use crate::db::{DeploymentDb, DeploymentRecord};
use crate::deploy::Deployer;
use crate::filter::Filter;
use crate::gitlab::GitLabClient;
use crate::notify::Notifier;
use crate::types::PushEvent;
use std::sync::Arc;
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
        cfg.harbor.clone(),
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
        // F3: per_page raised from 5 to 10; iterate ALL returned events (not
        // just the first) so multiple pushes between polls are not missed.
        match client.get_project_events(&encoded, 10).await {
            Ok(events) => {
                let new_last_id = events.first().map(|e| e.id);

                if last_event_id.is_none() {
                    // First poll: baseline only, never process events
                    if let Some(id) = new_last_id {
                        last_event_id = Some(id);
                        info!("Initial event ID: {}", id);
                    }
                } else {
                    // Subsequent polls: process new events, skipping already-seen
                    // IDs (<= last + break guards against GitLab ID rollback/ordering)
                    for event in &events {
                        let event_id = event.id;
                        if event_id <= last_event_id.unwrap_or(0) {
                            break;
                        }

                        // This mode is push-only (release is multi-only);
                        // the action=pushed filter means push_data should be set,
                        // but guard defensively anyway.
                        if event.push_data.is_none() {
                            continue;
                        }

                        let branch = event.push_data.as_ref()
                            .map(|pd| pd.r#ref.as_str())
                            .unwrap_or("");

                        if !filter.matches(project, branch) {
                            continue;
                        }

                        let push_event = match PushEvent::from_event(event, Some(project)) {
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
                        let cc_emails = cfg.notify.cc.clone();
                        let repo_emails = filter.repos.iter()
                            .find(|r| r.project == push_event.project)
                            .map(|r| r.emails.clone())
                            .unwrap_or_default();

                        tokio::spawn(async move {
                            let result = deployer
                                .deploy(&push_event, &gitlab_url, &gitlab_token, cancel_clone)
                                .await;

                            if result.cancelled {
                                return;
                            }

                            let record = DeploymentRecord {
                                id: 0,                  // DB 自增主键，insert 忽略
                                created_at: String::new(), // DB 默认 datetime('now')
                                project: push_event.project.clone(),
                                branch: push_event.branch.clone(),
                                commit_sha: push_event.commit.clone(),
                                author_name: push_event.author_name.clone(),
                                author_email: String::new(),
                                event_id: push_event.event_id,
                                event_type: "push".into(),
                                exit_code: result.exit_code,
                                status: if result.exit_code == 0 { "success".into() } else { "failed".into() },
                                stdout_tail: result.stdout.clone(),
                                stderr_tail: result.stderr.clone(),
                                duration_ms: result.duration.as_millis() as i64,
                            };

                            let _ = db.insert(&record).await;

                            if notify_author {
                                // F4: author email 优先；缺失时兜底 repo_emails ∪ cc（合并去重）
                                let email = crate::notify::merge_recipients(
                                    &client.get_user_email(push_event.author_id).await.unwrap_or_default(),
                                    &repo_emails,
                                    &cc_emails,
                                );
                                notifier.notify(&push_event, &result, &email).await;
                            }
                        });
                    }

                    if let Some(id) = new_last_id {
                        last_event_id = Some(id);
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
}
