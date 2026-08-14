//! Global events polling mode — monitors all visible projects.

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
        cfg.harbor.clone(),
    ));
    let notifier = Arc::new(Notifier::new(cfg.notify.msg_platform_url.clone())?);
    let db = Arc::new(DeploymentDb::open(&cfg.database.path).await?);

    let mut last_event_id: Option<u64> = None;
    let mut poll_count: u64 = 0;
    let mut active_tokens: Vec<CancellationToken> = Vec::new();

    info!("Global mode: monitoring all push events{} (interval {}s)",
        filter_info, cfg.gitlab.poll_interval_secs);

    let poll_interval = std::time::Duration::from_secs(cfg.gitlab.poll_interval_secs);

    loop {
        // F3: iterate ALL returned events (not just the first) so multiple
        // pushes between polls are not missed.
        match client.get_global_events(10).await {
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

                        // Push-only (release is multi-only); guard defensively.
                        if event.push_data.is_none() {
                            continue;
                        }

                        let project_path = match client.lookup_project(event.project_id).await {
                            Ok(Some(p)) => p,
                            _ => {
                                warn!("Could not lookup project ID {}", event.project_id);
                                continue;
                            }
                        };

                        let branch = event.push_data.as_ref()
                            .map(|pd| pd.r#ref.as_str())
                            .unwrap_or("");

                        if !filter.matches(&project_path, branch) {
                            continue;
                        }

                        let push_event = match PushEvent::from_event(event, Some(&project_path)) {
                            Some(pe) => pe,
                            None => {
                                warn!("Failed to parse push event for {}", project_path);
                                continue;
                            }
                        };

                        info!("Push detected: {} @ {} (commit: {:.8})",
                            push_event.project, push_event.branch, push_event.commit);

                        let cancel = CancellationToken::new();
                        let cancel_clone = cancel.clone();
                        let deployer = deployer.clone();
                        let client = client.clone();
                        let notifier = notifier.clone();
                        let db = db.clone();
                        let notify_author = cfg.notify.notify_author;

                        active_tokens.retain(|t| !t.is_cancelled());
                        active_tokens.push(cancel);

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
            active_tokens.retain(|t| !t.is_cancelled());
            info!("Heartbeat - {} polls [event_id={}] (active: {})", poll_count,
                last_event_id.map_or("?".into(), |id| id.to_string()), active_tokens.len());
        }

        tokio::time::sleep(poll_interval).await;
    }
}
