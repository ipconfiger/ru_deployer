//! Multi-project independent polling mode with parallel deployments.
//! Supports both push and release events.

use crate::config::Config;
use crate::db::{DeploymentDb, DeploymentRecord};
use crate::deploy::Deployer;
use crate::filter::Filter;
use crate::gitlab::GitLabClient;
use crate::notify::Notifier;
use crate::types::{GitLabRelease, PushEvent, ReleaseEvent};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Number of latest releases fetched per poll. Fetching more than one avoids
/// missing a prefix-matching release that sits behind a non-matching latest
/// release (multiple deployer instances sharing one repo).
const RELEASE_POLL_PAGE: u32 = 20;

/// Return the newest release whose tag matches `prefix`.
/// `releases` is newest-first (GitLab Releases API default order).
/// `None`/empty prefix → return the first (newest) release, preserving legacy behavior.
/// `Some(prefix)` → return the first release whose `tag_name` starts with `prefix`.
fn latest_matching_release<'a>(
    releases: &'a [GitLabRelease],
    prefix: Option<&str>,
) -> Option<&'a GitLabRelease> {
    match prefix {
        Some(p) if !p.is_empty() => releases.iter().find(|r| r.tag_name.starts_with(p)),
        _ => releases.first(),
    }
}

/// Per-project polling state.
struct ProjectState {
    encoded_path: String,
    last_event_id: Option<u64>,
    last_release_tag: Option<String>,
    short_name: String,
}

/// Active deployment per project:branch or project:release:tag.
#[allow(dead_code)]
struct ActiveDeploy {
    cancel_token: CancellationToken,
    join_handle: JoinHandle<()>,
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

    let active: Arc<DashMap<String, ActiveDeploy>> = Arc::new(DashMap::new());

    let mut state: HashMap<String, ProjectState> = HashMap::new();
    let projects: Vec<String> = filter.project_paths().map(String::from).collect();

    // Startup validation + restore last_event_id from DB
    for project in &projects {
        let exists = client.project_exists(project).await;
        if exists {
            info!("Filter project verified: {}", project);
        } else {
            warn!("Filter project NOT found or inaccessible: {} (will still monitor)", project);
        }
        let last_id = db.get_last_event_id(project).await.unwrap_or(None);
        if let Some(id) = last_id {
            info!("{} restored last event ID from DB: {}", project, id);
        }
        state.insert(
            project.clone(),
            ProjectState {
                encoded_path: project.replace('/', "%2F"),
                last_event_id: last_id,
                last_release_tag: None,
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
    let mut first_poll = true;  // Independent flag: always baseline on first poll, regardless of DB

    loop {
        for project in &projects {
            let (encoded_path, short_name) = {
                let s = match state.get(project) {
                    Some(s) => s,
                    None => continue,
                };
                (s.encoded_path.clone(), s.short_name.clone())
            };

            // === Release polling: baseline on first poll, detect new matching releases after ===
            if let Ok(releases) = client.get_releases(&encoded_path, RELEASE_POLL_PAGE).await {
                if let Some(r) = latest_matching_release(&releases, cfg.deploy.release_prefix.as_deref()) {
                    let current_tag = state.get(project).and_then(|s| s.last_release_tag.clone());
                    if first_poll {
                        info!("{} initial release tag: {}", short_name, r.tag_name);
                        state.get_mut(project).unwrap().last_release_tag = Some(r.tag_name.clone());
                    } else if current_tag.as_deref() != Some(&r.tag_name) {
                        info!("Release detected: {} tag={}", project, r.tag_name);
                        let release_event = ReleaseEvent {
                            project: project.clone(),
                            tag_name: r.tag_name.clone(),
                            author_name: r.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                            author_id: r.author.as_ref().map(|a| a.id).unwrap_or(0),
                            event_id: 0,
                        };
                        let deploy_key = format!("{}:release:{}", project, r.tag_name);
                        spawn_deploy(
                            deploy_key, None, Some(release_event),
                            &deployer, &client, &notifier, &db,
                            &cfg.gitlab.url, &cfg.gitlab.token,
                            cfg.notify.notify_author,
                            &active,
                            &filter,
                            &cfg.notify.cc,
                        );
                        state.get_mut(project).unwrap().last_release_tag = Some(r.tag_name.clone());
                    }
                }
            }

            match client.get_project_events_unfiltered(&encoded_path).await {
                Ok(events) => {
                    let new_last_id = events.first().map(|e| e.id);

                    // === FIRST POLL: only establish baseline, NEVER process events ===
                    if first_poll {
                        if let Some(id) = new_last_id {
                            info!("{} initial event ID: {}", short_name, id);
                            state.get_mut(project).unwrap().last_event_id = Some(id);
                        }
                        continue;
                    }

                    // === SUBSEQUENT POLLS: process new events ===
                    for latest in &events {
                        let event_id = latest.id;
                        // Skip already-processed events; unwrap_or(0) safe: GitLab event IDs start from 1
                        let last = state.get(project).and_then(|s| s.last_event_id).unwrap_or(0);
                        if event_id <= last {
                            break;
                        }

                        // --- Push event ---
                        if latest.push_data.is_some() {
                            let branch = latest.push_data.as_ref()
                                .map(|pd| pd.r#ref.as_str())
                                .unwrap_or("");

                            if !filter.matches(project, branch) {
                                continue;
                            }

                            let push_event = match PushEvent::from_event(latest, Some(project)) {
                                Some(pe) => pe,
                                None => { warn!("Failed to parse push event for {}", project); continue; }
                            };

                            info!("Push detected: {} @ {} (commit: {:.8})",
                                push_event.project, push_event.branch, push_event.commit);

                            let deploy_key = format!("{}:{}", push_event.project, push_event.branch);
                            spawn_deploy(
                                deploy_key, Some(push_event), None,
                                &deployer, &client, &notifier, &db,
                                &cfg.gitlab.url, &cfg.gitlab.token,
                                cfg.notify.notify_author,
                                &active,
                                &filter,
                                &cfg.notify.cc,
                            );
                        }
                    }

                    // Update last_event_id
                    if let Some(id) = new_last_id {
                        state.get_mut(project).unwrap().last_event_id = Some(id);
                    }
                }
                Err(e) => {
                    error!("{} API request failed: {}", short_name, e);
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        // After first full loop completes, enable event processing
        first_poll = false;

        poll_count += 1;
        if poll_count % 30 == 0 {
            let ids: Vec<String> = state.values()
                .map(|s| format!("{}={}", s.short_name, s.last_event_id.map_or("?".into(), |id| id.to_string())))
                .collect();
            info!("Heartbeat - {} polls [{}] (active: {})", poll_count, ids.join(", "), active.len());
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Spawn a deploy or release task with cancellation support.
fn spawn_deploy(
    key: String,
    push: Option<PushEvent>,
    release: Option<ReleaseEvent>,
    deployer: &Arc<Deployer>,
    client: &Arc<GitLabClient>,
    notifier: &Arc<Notifier>,
    db: &Arc<DeploymentDb>,
    gitlab_url: &str,
    gitlab_token: &str,
    notify_author: bool,
    active: &Arc<DashMap<String, ActiveDeploy>>,
    filter: &Filter,
    cc_emails: &[String],
) {
    // Cancel previous deployment for same key
    if let Some((_k, old)) = active.remove(&key) {
        info!("Cancelling previous deployment for {}", key);
        old.cancel_token.cancel();
    }

    let deployer = deployer.clone();
    let client = client.clone();
    let notifier = notifier.clone();
    let db = db.clone();
    let active_map = active.clone();
    let gitlab_url = gitlab_url.to_string();
    let gitlab_token = gitlab_token.to_string();
    let cc_emails = cc_emails.to_vec();
    let repo_emails = if let Some(ref re) = release {
        filter.repos.iter()
            .find(|r| r.project == re.project)
            .map(|r| r.emails.clone())
            .unwrap_or_default()
    } else if let Some(ref pe) = push {
        filter.repos.iter()
            .find(|r| r.project == pe.project)
            .map(|r| r.emails.clone())
            .unwrap_or_default()
    } else {
        vec![]
    };

    let token = CancellationToken::new();
    let token_clone = token.clone();

    let key_for_outer = key.clone();
    let key_inner = key_for_outer.clone();
    let handle = tokio::spawn(async move {
        let (result, author_id, event_type, project_name, branch_or_tag, gitlab_event_id, commit_sha, author_name) = if let Some(ref pe) = push {
            let result = deployer.deploy(pe, &gitlab_url, &gitlab_token, token_clone).await;
            let commit = result.commit.clone();
            let name = pe.author_name.clone();
            (result, pe.author_id, "push".to_string(), pe.project.clone(), pe.branch.clone(), pe.event_id, commit, name)
        } else if let Some(ref re) = release {
            let result = deployer.release(re, &gitlab_url, &gitlab_token, token_clone).await;
            let commit = result.commit.clone();
            let name = re.author_name.clone();
            (result, re.author_id, "release".to_string(), re.project.clone(), re.tag_name.clone(), re.event_id, commit, name)
        } else {
            return;
        };

        if result.cancelled {
            info!("Deployment {} was cancelled", key);
            active_map.remove(&key);
            return;
        }

        let record = DeploymentRecord {
            project: project_name.clone(),
            branch: branch_or_tag.clone(),
            commit_sha,
            author_name,
            author_email: String::new(),
            event_id: gitlab_event_id,
            event_type,
            exit_code: result.exit_code,
            status: if result.exit_code == 0 { "success".into() } else { "failed".into() },
            stdout_tail: result.stdout.clone(),
            stderr_tail: result.stderr.clone(),
            duration_ms: result.duration.as_millis() as i64,
        };

                        if let Err(e) = db.insert(&record).await {
                            error!("Failed to record deployment: {:?}", e);
                        }

        if notify_author {
            let mut email = client.get_user_email(author_id).await.unwrap_or_default();
            if email.is_empty() && !repo_emails.is_empty() {
                email = repo_emails.join(",");
            } else if email.is_empty() && !cc_emails.is_empty() {
                email = cc_emails.join(",");
            }
            if let Some(ref pe) = push {
                notifier.notify(pe, &result, &email).await;
            } else if let Some(ref re) = release {
                notifier.notify_release(re, &result, &email).await;
            }
        }

        active_map.remove(&key_inner);
    });

    active.insert(key_for_outer, ActiveDeploy { cancel_token: token, join_handle: handle });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> GitLabRelease {
        GitLabRelease {
            tag_name: tag.to_string(),
            name: String::new(),
            created_at: None,
            author: None,
            commit: None,
        }
    }

    #[test]
    fn latest_matching_release_no_prefix_returns_first() {
        let releases = vec![release("gpu-2"), release("gpu-1"), release("app-1")];
        let r = latest_matching_release(&releases, None);
        assert_eq!(r.map(|r| r.tag_name.as_str()), Some("gpu-2"));
    }

    #[test]
    fn latest_matching_release_empty_prefix_returns_first() {
        let releases = vec![release("gpu-2"), release("app-1")];
        let r = latest_matching_release(&releases, Some(""));
        assert_eq!(r.map(|r| r.tag_name.as_str()), Some("gpu-2"));
    }

    #[test]
    fn latest_matching_release_with_matching_prefix() {
        let releases = vec![release("app-2"), release("gpu-2"), release("gpu-1")];
        let r = latest_matching_release(&releases, Some("gpu-"));
        assert_eq!(r.map(|r| r.tag_name.as_str()), Some("gpu-2"));
    }

    #[test]
    fn latest_matching_release_no_match_returns_none() {
        let releases = vec![release("app-2"), release("app-1")];
        let r = latest_matching_release(&releases, Some("gpu-"));
        assert_eq!(r.map(|r| r.tag_name.as_str()), None);
    }

    #[test]
    fn latest_matching_release_empty_list_returns_none() {
        let releases: Vec<GitLabRelease> = vec![];
        assert_eq!(latest_matching_release(&releases, None).is_none(), true);
        assert_eq!(latest_matching_release(&releases, Some("gpu-")).is_none(), true);
    }
}
