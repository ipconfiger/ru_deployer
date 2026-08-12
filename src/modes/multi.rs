//! Multi-project independent polling mode (default).
//!
//! Each project in the filter config is polled independently via the Events API,
//! with its own last_event_id tracker.

use crate::config::Config;
use crate::db::{DeploymentDb, DeploymentRecord};
use crate::deploy::{DeployResult, Deployer};
use crate::filter::Filter;
use crate::gitlab::GitLabClient;
use crate::notify::Notifier;
use crate::types::PushEvent;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

/// Per-project polling state.
struct ProjectState {
    encoded_path: String,
    last_event_id: Option<u64>,
    short_name: String,
}

pub async fn watch_multi(cfg: &Config) -> anyhow::Result<()> {
    let filter = Filter::load(cfg.filter.file.as_deref())?;
    if filter.is_empty() {
        anyhow::bail!("Multi-project mode requires a filter configuration (--filter)");
    }

    // Initialize clients
    let client = Arc::new(GitLabClient::new(cfg.gitlab.url.clone(), cfg.gitlab.token.clone())?);
    let deployer = Arc::new(Deployer::new(
        cfg.deploy.src_dir.clone(),
        cfg.deploy.scripts_dir.clone(),
        cfg.deploy.script_timeout_secs,
        cfg.harbor.password.clone(),
    ));
    let notifier = Arc::new(Notifier::new(cfg.notify.msg_platform_url.clone())?);
    let db = Arc::new(DeploymentDb::open(&cfg.database.path).await?);

    // Build project state map
    let mut state: HashMap<String, ProjectState> = HashMap::new();
    let projects: Vec<String> = filter.project_paths().map(String::from).collect();

    // Startup validation: check each project exists
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
        // Check for graceful shutdown
        if signal::ctrl_c().await.is_ok() {
            info!("SIGINT received, shutting down...");
            break;
        }

        for project in &projects {
            // Get current state values (immutable borrow, dropped after this scope)
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
                            // First poll: record baseline
                            state.get_mut(project).unwrap().last_event_id = Some(event_id);
                            info!("{} initial event ID: {}", short_name, event_id);
                        } else if event_id != last_id.unwrap() {
                            // New event detected
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

                            info!(
                                "Push detected: {} @ {} (commit: {:.8})",
                                push_event.project,
                                push_event.branch,
                                push_event.commit
                            );

                            // Deploy
                            let result = deployer
                                .deploy(&push_event, &cfg.gitlab.url, &cfg.gitlab.token)
                                .await
                                .unwrap_or_else(|e| DeployResult {
                                    exit_code: -1,
                                    stdout: String::new(),
                                    stderr: format!("{}", e),
                                    commit: String::new(),
                                    duration: std::time::Duration::ZERO,
                                });

                            // Record to DB
                            let record = DeploymentRecord {
                                project: push_event.project.clone(),
                                branch: push_event.branch.clone(),
                                commit_sha: push_event.commit.clone(),
                                author_name: push_event.author_name.clone(),
                                author_email: String::new(), // filled below
                                event_id: push_event.event_id,
                                exit_code: result.exit_code,
                                status: if result.exit_code == 0 {
                                    "success".into()
                                } else {
                                    "failed".into()
                                },
                                stdout_tail: result.stdout.clone(),
                                stderr_tail: result.stderr.clone(),
                                duration_ms: result.duration.as_millis() as i64,
                            };

                            if let Err(e) = db.insert(&record).await {
                                error!("Failed to record deployment: {}", e);
                            }

                            // Send notification
                            if cfg.notify.notify_author {
                                let email = client
                                    .get_user_email(push_event.author_id)
                                    .await
                                    .unwrap_or_default();
                                notifier.notify(&push_event, &result, &email).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("{} API request failed: {}", short_name, e);
                }
            }

            // Brief pause between project polls to avoid thundering herd
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        poll_count += 1;
        if poll_count % 30 == 0 {
            let ids: Vec<String> = state
                .values()
                .map(|s| format!("{}={}", s.short_name, s.last_event_id.map_or("?".into(), |id| id.to_string())))
                .collect();
            info!("Heartbeat - {} polls [{}]", poll_count, ids.join(", "));
        }

        tokio::time::sleep(poll_interval).await;
    }

    info!("ru_deployer multi-mode shut down");
    Ok(())
}
