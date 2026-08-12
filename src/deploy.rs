//! Deployment orchestration: git pull → execute deploy script → capture output.

use crate::git::GitRepo;
use crate::types::PushEvent;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Result of a deployment execution.
#[derive(Debug, Clone)]
pub struct DeployResult {
    pub exit_code: i32,
    pub stdout: String,   // head 2KB + tail 8KB
    pub stderr: String,   // head 2KB + tail 8KB
    pub commit: String,
    pub duration: Duration,
}

/// Maximum captured output size
const OUTPUT_MAX: usize = 10 * 1024; // 10KB
const HEAD_SIZE: usize = 2 * 1024;   // 2KB
const TAIL_SIZE: usize = OUTPUT_MAX - HEAD_SIZE; // 8KB

fn truncate_output(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() <= OUTPUT_MAX {
        return raw.to_string();
    }
    let head: String = chars.iter().take(HEAD_SIZE).collect();
    let tail_start = chars.len().saturating_sub(TAIL_SIZE);
    let tail: String = chars.iter().skip(tail_start).collect();
    format!("{}\n... [{} bytes truncated] ...\n{}",
        head, raw.len() - OUTPUT_MAX, tail)
}

/// Manages deployment orchestration with per-project+branch serialization.
pub struct Deployer {
    git: GitRepo,
    scripts_dir: PathBuf,
    script_timeout: Duration,
    harbor_password: String,
    /// Per-(project:branch) mutex for serializing deployments
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Deployer {
    pub fn new(
        src_dir: PathBuf,
        scripts_dir: PathBuf,
        script_timeout_secs: u64,
        harbor_password: String,
    ) -> Self {
        Self {
            git: GitRepo::new(src_dir),
            scripts_dir,
            script_timeout: Duration::from_secs(script_timeout_secs),
            harbor_password,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Execute a full deployment for a push event.
    pub async fn deploy(
        &self,
        event: &PushEvent,
        gitlab_host: &str,
        gitlab_token: &str,
    ) -> Result<DeployResult> {
        let lock_key = format!("{}:{}", event.project, event.branch);

        // Get or create the per-project+branch lock
        let lock = {
            let mut map = self.locks.lock().await;
            map.entry(lock_key.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };

        // Serialize deployments for the same project+branch
        let _guard = lock.lock().await;
        info!("Starting deployment for {} @ {}", event.project, event.branch);

        self.do_deploy(event, gitlab_host, gitlab_token).await
    }

    async fn do_deploy(
        &self,
        event: &PushEvent,
        gitlab_host: &str,
        gitlab_token: &str,
    ) -> Result<DeployResult> {
        let start = Instant::now();

        // 1. Git pull
        info!("Pulling code for {} @ {}", event.project, event.branch);
        let repo_path = self
            .git
            .ensure(gitlab_host, gitlab_token, &event.project, &event.branch)
            .await
            .with_context(|| {
                format!(
                    "Failed to pull code for {} @ {}",
                    event.project, event.branch
                )
            })?;

        let commit = self.git.current_commit(&repo_path).await?;

        // 2. Find deploy script
        let short_name = event.project.rsplit('/').next().unwrap_or(&event.project);
        let script_name = format!("{}_deploy.sh", short_name);
        let script_path = self.scripts_dir.join(&script_name);

        if !script_path.exists() {
            warn!("Deploy script not found: {}", script_path.display());
            return Ok(DeployResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Deploy script not found: {}", script_path.display()),
                commit,
                duration: start.elapsed(),
            });
        }

        // 3. Execute script with timeout
        info!("Executing deploy script: {}", script_path.display());
        let output_result = tokio::time::timeout(self.script_timeout, async {
            Command::new("bash")
                .arg(&script_path)
                .current_dir(&self.scripts_dir)
                .env("GITLAB_PROJECT", &event.project)
                .env("GITLAB_BRANCH", &event.branch)
                .env("GITLAB_COMMIT", &event.commit)
                .env("GITLAB_AUTHOR", &event.author_name)
                .env("GITLAB_EVENT_ID", event.event_id.to_string())
                .env("SRC_DIR", &repo_path)
                .env("SCRIPTS_DIR", &self.scripts_dir)
                .env("HARBOR_PASSWORD", &self.harbor_password)
                .output()
                .await
        })
        .await;

        let (exit_code, stdout, stderr) = match output_result {
            Ok(Ok(output)) => {
                let code = output.status.code().unwrap_or(-1);
                let out = String::from_utf8_lossy(&output.stdout).to_string();
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                (code, out, err)
            }
            Ok(Err(e)) => {
                error!("Deploy script execution error: {}", e);
                (-1, String::new(), format!("Failed to execute deploy script: {}", e))
            }
            Err(_elapsed) => {
                error!(
                    "Deploy script timed out after {}s for {} @ {}",
                    self.script_timeout.as_secs(),
                    event.project,
                    event.branch
                );
                (-1, String::new(), format!("Script timed out after {}s", self.script_timeout.as_secs()))
            }
        };

        let duration = start.elapsed();

        let stdout_trunc = truncate_output(&stdout);
        let stderr_trunc = truncate_output(&stderr);

        if exit_code == 0 {
            info!(
                "Deployment succeeded for {} @ {} ({}s)",
                event.project,
                event.branch,
                duration.as_secs()
            );
        } else {
            warn!(
                "Deployment failed for {} @ {} (exit={}, {}s)",
                event.project,
                event.branch,
                exit_code,
                duration.as_secs()
            );
        }

        Ok(DeployResult {
            exit_code,
            stdout: stdout_trunc,
            stderr: stderr_trunc,
            commit,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_output_short() {
        let result = truncate_output("hello");
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "A".repeat(15 * 1024);
        let result = truncate_output(&long);
        assert!(result.len() <= OUTPUT_MAX + 100); // allow for truncation message overhead
        assert!(result.contains("["));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_output_max_constant() {
        assert_eq!(HEAD_SIZE + TAIL_SIZE, OUTPUT_MAX);
    }
}
