//! GitLab REST API client for polling push events and user info.

use crate::types::{Commit, Event, GitLabRelease, GitLabUser};
use anyhow::{Context, Result};
use reqwest::Client;
use tracing::{debug, warn};

/// Client for the GitLab REST API.
#[derive(Debug, Clone)]
pub struct GitLabClient {
    base_url: String,
    token: String,
    client: Client,
}

impl GitLabClient {
    /// Create a new GitLab API client.
    pub fn new(base_url: String, token: String) -> Result<Self> {
        let client = Client::builder()
            .danger_accept_invalid_certs(true) // Allow self-signed certs on internal network
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            client,
        })
    }

    /// Build request headers with auth token.
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if !self.token.is_empty() {
            headers.insert(
                "PRIVATE-TOKEN",
                reqwest::header::HeaderValue::from_str(&self.token).unwrap(),
            );
        }
        headers
    }

    // === Events API ===

    /// Fetch project-level push events.
    /// `project_path` should be URL-encoded (e.g., "dev-team%2Fapi").
    pub async fn get_project_events(
        &self,
        project_path: &str,
        per_page: u32,
    ) -> Result<Vec<Event>> {
        let url = format!("{}/api/v4/projects/{}/events", self.base_url, project_path);
        debug!("GET {}", url);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&[
                ("action", "pushed"),
                ("per_page", &per_page.to_string()),
            ])
            .send()
            .await
            .with_context(|| format!("Failed to fetch events for project: {}", project_path))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("Authentication failed — check GITLAB_TOKEN");
        }
        resp.error_for_status_ref()?;

        let events: Vec<Event> = resp.json().await.context("Failed to parse events JSON")?;
        debug!("Got {} events for project {}", events.len(), project_path);
        Ok(events)
    }

    /// Fetch global push events (all visible projects).
    pub async fn get_global_events(&self, per_page: u32) -> Result<Vec<Event>> {
        let url = format!("{}/api/v4/events", self.base_url);
        debug!("GET {}", url);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&[
                ("action", "pushed"),
                ("per_page", &per_page.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch global events")?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("Authentication failed — check GITLAB_TOKEN");
        }
        resp.error_for_status_ref()?;

        let events: Vec<Event> = resp.json().await.context("Failed to parse events JSON")?;
        debug!("Got {} global events", events.len());
        Ok(events)
    }

    /// Fetch project events without action filter (for detecting both push + release).
    /// Uses per_page=10 to avoid missing events when interleaved with issue/comment events.
    pub async fn get_project_events_unfiltered(
        &self,
        project_path: &str,
    ) -> Result<Vec<Event>> {
        let url = format!("{}/api/v4/projects/{}/events", self.base_url, project_path);
        debug!("GET {} (unfiltered)", url);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&[("per_page", "10")])
            .send()
            .await
            .with_context(|| format!("Failed to fetch events for project: {}", project_path))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("Authentication failed — check GITLAB_TOKEN");
        }
        resp.error_for_status_ref()?;

        let events: Vec<Event> = resp.json().await.context("Failed to parse events JSON")?;
        debug!("Got {} events (unfiltered) for project {}", events.len(), project_path);
        Ok(events)
    }

    // === Commits API ===

    /// Fetch the latest commits for a branch.
    pub async fn get_commits(
        &self,
        project_path: &str,
        branch: &str,
        per_page: u32,
    ) -> Result<Vec<Commit>> {
        let url = format!(
            "{}/api/v4/projects/{}/repository/commits",
            self.base_url, project_path
        );
        debug!("GET {} (ref={})", url, branch);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&[
                ("ref_name", branch),
                ("per_page", &per_page.to_string()),
            ])
            .send()
            .await
            .with_context(|| format!("Failed to fetch commits for project: {}", project_path))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("Authentication failed — check GITLAB_TOKEN");
        }
        resp.error_for_status_ref()?;

        let commits: Vec<Commit> = resp.json().await.context("Failed to parse commits JSON")?;
        debug!("Got {} commits for {}/{}", commits.len(), project_path, branch);
        Ok(commits)
    }

    // === Project Lookup ===

    /// Look up a project path by numeric project ID.
    pub async fn lookup_project(&self, project_id: u64) -> Result<Option<String>> {
        let url = format!("{}/api/v4/projects/{}", self.base_url, project_id);
        debug!("GET {}", url);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to lookup project ID: {}", project_id))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            warn!("Project ID {} not found", project_id);
            return Ok(None);
        }
        resp.error_for_status_ref()?;

        let project: serde_json::Value = resp.json().await?;
        let path = project["path_with_namespace"].as_str().map(String::from);
        Ok(path)
    }

    /// Validate that a project exists (for startup filter validation).
    pub async fn project_exists(&self, project_path: &str) -> bool {
        let encoded = project_path.replace('/', "%2F");
        let url = format!("{}/api/v4/projects/{}", self.base_url, encoded);
        debug!("GET {} (validation)", url);

        match self.client.get(&url).headers(self.headers()).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(e) => {
                warn!("Project validation failed for {}: {}", project_path, e);
                false
            }
        }
    }

    // === User API ===

    /// Get a user's email by user ID.
    pub async fn get_user_email(&self, user_id: u64) -> Result<String> {
        let url = format!("{}/api/v4/users/{}", self.base_url, user_id);
        debug!("GET {}", url);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch user ID: {}", user_id))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(String::new());
        }
        resp.error_for_status_ref()?;

        let user: GitLabUser = resp.json().await?;
        Ok(if !user.email.is_empty() {
            user.email
        } else {
            user.public_email
        })
    }

    // === Releases API ===

    /// Fetch latest releases for a project.
    pub async fn get_releases(&self, project_path: &str, per_page: u32) -> Result<Vec<GitLabRelease>> {
        let url = format!("{}/api/v4/projects/{}/releases", self.base_url, project_path);
        debug!("GET {} (releases)", url);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(&[("per_page", &per_page.to_string())])
            .send()
            .await
            .with_context(|| format!("Failed to fetch releases for project: {}", project_path))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("Authentication failed — check GITLAB_TOKEN");
        }
        resp.error_for_status_ref()?;

        let releases: Vec<GitLabRelease> = resp.json().await
            .context("Failed to parse releases JSON")?;
        debug!("Got {} releases for project {}", releases.len(), project_path);
        Ok(releases)
    }
}
