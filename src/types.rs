//! Shared data types for GitLab API responses and deployment events.

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// A GitLab push event from the Events API.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: u64,
    pub project_id: u64,
    pub action_name: String,
    pub target_title: Option<String>,
    pub target_type: Option<String>,
    pub author_id: u64,
    pub author: Option<Author>,
    pub push_data: Option<PushData>,
    pub project: Option<Project>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Push-specific data within an event.
#[derive(Debug, Clone, Deserialize)]
pub struct PushData {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub ref_type: String,
    #[serde(default)]
    pub r#ref: String,
    #[serde(default)]
    pub commit_to: String,
    #[serde(default)]
    pub commit_count: u32,
    #[serde(default)]
    pub commit_title: String,
}

/// GitLab user/author info.
#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
}

/// GitLab project summary.
#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub path_with_namespace: String,
    #[serde(default)]
    pub name: String,
}

/// A commit from the Commits API.
#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub short_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub author_email: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub web_url: String,
}

/// User info from the Users API.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabUser {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub public_email: String,
}

/// A high-level push event, normalized from Event API data.
#[derive(Debug, Clone)]
pub struct PushEvent {
    pub event_id: u64,
    pub project_id: u64,
    pub project: String,
    pub branch: String,
    pub commit: String,
    pub author_name: String,
    pub author_id: u64,
    pub created_at: Option<DateTime<Utc>>,
}

impl PushEvent {
    /// Build a PushEvent from a GitLab Event, optionally looking up the project path.
    pub fn from_event(event: &Event, project_name: Option<&str>) -> Option<Self> {
        let push_data = event.push_data.as_ref()?;
        let project = project_name
            .map(String::from)
            .or_else(|| event.project.as_ref().map(|p| p.path_with_namespace.clone()))
            .or_else(|| event.target_title.clone())?;

        let branch = push_data.r#ref.trim_start_matches("refs/heads/").to_string();
        if branch.is_empty() {
            return None;
        }

        Some(Self {
            event_id: event.id,
            project_id: event.project_id,
            project,
            branch,
            commit: push_data.commit_to.clone(),
            author_name: event
                .author
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "unknown".into()),
            author_id: event.author_id,
            created_at: event.created_at,
        })
    }
}
