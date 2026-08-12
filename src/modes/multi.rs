//! Multi-project independent polling mode with parallel deployments.

use crate::config::Config;
use crate::db::{DeploymentDb, DeploymentRecord};
use crate::deploy::Deployer;
use crate::filter::Filter;
use crate::gitlab::GitLabClient;
use crate::notify::Notifier;
use crate::types::PushEvent;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::signal;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Per-project polling state.
struct ProjectState {
    encoded_path: String,
    last_event_id: Option<u64>,
    short_name: String,
}

/// Active deployment (abortable) per project:branch.
struct ActiveDeploy {
    cancel_token: CancellationToken,
    #[allow(dead_code)]
    join_handle: JoinHandle<()>, // keep alive to detect completion
}

pub async fn watch_multi(cfg: &Config) -> anyhow::Result<()> {
    let filter = Filter::load(cfg.filter.file.as_deref())?;
    if filter.is_empty() {
        anyhow::bail!("Multi-project mode requires a filter configuration (--filter)");
    }

    let client = Arc::new(GitLabClient::new(cfg.gitlab.url.clone(), cfg.gitlab.token.clone())?);
    let deployer = Arc::new(Deployer::new(
        cfg.deploy.src_dir.clone(),
        cfg.deploy.scripts_dir.clone(),
        cfg.deploy.script_timeout_secs,
        cfg.harbor.password.clone(),
    ));
    let notifier = Arc::new(Notifier::new(cfg.notify.msg_platform_url.clone())?);
    let db = Arc::new(DeploymentDb::open(&cfg.database.path).await?);

    // Active deployments: key = "project:branch"
    let active: Arc<DashMap<String, ActiveDeploy>> = Arc::new(DashMap::new());

    // Build project state map
    let mut state: HashMap<String, ProjectState> = HashMap::new();
    let projects: Vec<String> = filter.project_paths().map(String::from).collect();

    // Startup validation
    for project in &projects {
        let exists = client.project_exists(project).await;
        if exists {
            info!("Filter project verified: {}", project);
        } else {
            warn!("Filter project NOT found or inaccessible: {} (will still monitor)", project);
        }
        state.insert(
            project.clone(),
            ProjectState {
                encoded_path: project.replace('/', "%2F"),
                last_event_id: None,
                short_name: project.rsplit('/').next().unwrap_or(project).to_string(),
            },
        );
    }

    info!("Multi-project mode: monitoring {} project(s)", projects.len());
    for p in &projects {
        let repo = filter.repos.iter().find(|r| r.project == *p);
        let branches = repo.map(|r| r.branches.as_slice()).unwrap_or(&[]);
        let branch_info = if branches.is_empty() { "all".into() } else { branches.join(", ") };
        info!("  - {} (branches: {})", p, branch_info);
    }

    let poll_interval = std::time::Duration::from_secs(cfg.gitlab.poll_interval_secs);
    let mut poll_count: u64 = 0;

    loop {
        if signal::ctrl_c().await.is_ok() {
            info!("SIGINT received, shutting down...");
            break;
        }

        for project in &projects {
            let (encoded_path, short_name, last_id, is_initial) = {
                let s = match state.get(project) {
                    Some(s) => s,
                    None => continue,
                };
                (s.encoded_path.clone(), s.short_name.clone(), s.last_event_id, s.last_event_id.is_none())
            };

            match client.get_project_events(&encoded_path, 3).await {
                Ok(events) => {
                    if let Some(latest) = events.first() {
                        let event_id = latest.id;

                        if is_initial {
                            state.get_mut(project).unwrap().last_event_id = Some(event_id);
                            info!("{} initial event ID: {}", short_name, event_id);
                        } else if event_id != last_id.unwrap() {
                            state.get_mut(project).unwrap().last_event_id = Some(event_id);

                            let branch = latest
                                .push_data
                                .as_ref()
                                .map(|pd| pd.r#ref.as_str())
                                .unwrap_or("");

                            if !filter.matches(project, branch) {
                                continue;
                            }

                            let push_event = match PushEvent::from_event(latest, Some(project)) {
                                Some(pe) => pe,
                                None => {
                                    warn!("Failed to parse push event for {}", project);
                                    continue;
                                }
                            };

                            let deploy_key = format!("{}:{}", push_event.project, push_event.branch);

                            // Cancel previous deployment for same project:branch
                            if let Some((_k, old)) = active.remove(&deploy_key) {
                                info!("Cancelling previous deployment for {}", deploy_key);
                                old.cancel_token.cancel();
                            }

                            // Spawn new deployment (non-blocking)
                            let deployer = deployer.clone();
                            let client = client.clone();
                            let notifier = notifier.clone();
                            let db = db.clone();
                            let notify_author = cfg.notify.notify_author;
                            let active_map = active.clone();
                            let token = CancellationToken::new();
                            let token_clone = token.clone();
                            let key = deploy_key.clone();

                            let gitlab_url = cfg.gitlab.url.clone();
                            let gitlab_token = cfg.gitlab.token.clone();

                            let handle = tokio::spawn(async move {
                                let result = deployer
                                    .deploy(&push_event, &gitlab_url, &gitlab_token, token_clone)
                                    .await;

                                if result.cancelled {
                                    info!("Deployment {} was cancelled", key);
                                    active_map.remove(&key);
                                    return;
                                }

                                // Record to DB
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

                                if let Err(e) = db.insert(&record).await {
                                    error!("Failed to record deployment: {}", e);
                                }

                                // Send notification
                                if notify_author {
                                    let email = client.get_user_email(push_event.author_id).await.unwrap_or_default();
                                    notifier.notify(&push_event, &result, &email).await;
                                }

                                // Clean up active entry
                                active_map.remove(&key);
                            });

                            active.insert(deploy_key, ActiveDeploy {
                                cancel_token: token,
                                join_handle: handle,
                            });
                        }
                    }
                }
                Err(e) => {
                    error!("{} API request failed: {}", short_name, e);
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        poll_count += 1;
        if poll_count % 30 == 0 {
            let ids: Vec<String> = state
                .values()
                .map(|s| format!("{}={}", s.short_name, s.last_event_id.map_or("?".into(), |id| id.to_string())))
                .collect();
            let active_count = active.len();
            info!("Heartbeat - {} polls [{}] (active deploys: {})", poll_count, ids.join(", "), active_count);
        }

        tokio::time::sleep(poll_interval).await;
    }

    info!("ru_deployer multi-mode shut down");
    Ok(())
}
