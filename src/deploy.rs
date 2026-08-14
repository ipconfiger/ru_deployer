//! Deployment orchestration: git pull → execute deploy script → capture output.
//!
//! Supports cancellation: when a new push arrives for the same project+branch,
//! the running deployment is cancelled and its child process killed.

use crate::config::HarborConfig;
use crate::git::GitRepo;
use crate::types::{PushEvent, ReleaseEvent};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Result of a deployment execution.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DeployResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub commit: String,
    pub duration: Duration,
    pub cancelled: bool,
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
    format!(
        "{}\n... [{} bytes truncated] ...\n{}",
        head,
        raw.len() - OUTPUT_MAX,
        tail
    )
}

/// Manages deployment orchestration with cancellation support.
#[derive(Debug, Clone)]
pub struct Deployer {
    git: GitRepo,
    scripts_dir: PathBuf,
    script_timeout: Duration,
    /// Full Harbor config (registry/project/user/password), injected into
    /// child scripts as HARBOR_* env vars (T1: config-driven, no hardcoding).
    harbor: HarborConfig,
}

impl Deployer {
    pub fn new(
        src_dir: PathBuf,
        scripts_dir: PathBuf,
        script_timeout_secs: u64,
        harbor: HarborConfig,
    ) -> Self {
        Self {
            git: GitRepo::new(src_dir),
            scripts_dir,
            script_timeout: Duration::from_secs(script_timeout_secs),
            harbor,
        }
    }

    /// Execute a full deployment. If `cancel_token` fires, the running child
    /// process is killed and the deployment returns with `cancelled: true`.
    pub async fn deploy(
        &self,
        event: &PushEvent,
        gitlab_host: &str,
        gitlab_token: &str,
        cancel_token: CancellationToken,
    ) -> DeployResult {
        info!(
            "Starting deployment for {} @ {}",
            event.project, event.branch
        );

        let script_name = format!("{}_deploy.sh", event.project.rsplit('/').next().unwrap_or(&event.project));
        self.do_run(&event.project, &event.branch, &script_name, gitlab_host, gitlab_token, cancel_token, false).await
    }

    /// Execute a release build.
    pub async fn release(
        &self,
        event: &ReleaseEvent,
        gitlab_host: &str,
        gitlab_token: &str,
        cancel_token: CancellationToken,
    ) -> DeployResult {
        info!(
            "Starting release build for {} @ {}",
            event.project, event.tag_name
        );

        let script_name = format!("{}_release.sh", event.project.rsplit('/').next().unwrap_or(&event.project));
        self.do_run(&event.project, &event.tag_name, &script_name, gitlab_host, gitlab_token, cancel_token, true).await
    }

    /// Shared implementation for deploy and release.
    /// `ref_name` is branch for deploy, tag for release.
    async fn do_run(
        &self,
        project: &str,
        ref_name: &str,
        script_filename: &str,
        gitlab_host: &str,
        gitlab_token: &str,
        cancel_token: CancellationToken,
        is_tag: bool,
    ) -> DeployResult {
        let start = Instant::now();

        // 1. Git pull (branch or tag)
        let repo_path = if is_tag {
            match self.git.ensure_tag(gitlab_host, gitlab_token, project, ref_name).await {
                Ok(p) => p,
                Err(e) => {
                    return DeployResult {
                        exit_code: -1, stdout: String::new(),
                        stderr: format!("Failed to pull code: {}", e),
                        commit: String::new(), duration: start.elapsed(), cancelled: false,
                    };
                }
            }
        } else {
            match self.git.ensure(gitlab_host, gitlab_token, project, ref_name).await {
                Ok(p) => p,
                Err(e) => {
                    return DeployResult {
                        exit_code: -1, stdout: String::new(),
                        stderr: format!("Failed to pull code: {}", e),
                        commit: String::new(), duration: start.elapsed(), cancelled: false,
                    };
                }
            }
        };

        let commit = self.git.current_commit(&repo_path).await.unwrap_or_default();
        // Make repo_path absolute so the script can cd to it regardless of current_dir
        let _repo_path = repo_path.canonicalize().unwrap_or(repo_path);

        // 2. Find deploy script
        let script_name = script_filename.to_string();
        let script_path = self.scripts_dir.join(&script_name);

        if !script_path.exists() {
            warn!("Deploy script not found: {}", script_path.display());
            return DeployResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Deploy script not found: {}", script_path.display()),
                commit,
                duration: start.elapsed(),
                cancelled: false,
            };
        }

        // 3. Execute script with cancellation + timeout
        info!("Executing deploy script: {}", script_path.display());
        let result = self
            .run_script(&script_path, ref_name, cancel_token)
            .await;

        let (exit_code, stdout_raw, stderr_raw) = match result {
            ScriptResult::Completed { code, stdout, stderr } => {
                let duration = start.elapsed();
                if code == 0 {
                    info!(
                        "Deployment succeeded for {} @ {} ({}s)",
                        project, ref_name, duration.as_secs()
                    );
                } else {
                    warn!(
                        "Deployment failed for {} @ {} (exit={}, {}s)",
                        project, ref_name, code, duration.as_secs()
                    );
                }
                // Always log script output so users can see build progress
                if !stderr.is_empty() {
                    info!("Script stderr:\n{}", stderr);
                }
                if !stdout.is_empty() {
                    info!("Script stdout:\n{}", stdout);
                }
                (code, stdout, stderr)
            }
            ScriptResult::Cancelled => {
                info!(
                    "Deployment cancelled for {} @ {}",
                    project, ref_name
                );
                return DeployResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: "Deployment cancelled by newer push".into(),
                    commit,
                    duration: start.elapsed(),
                    cancelled: true,
                };
            }
            ScriptResult::Timeout => {
                error!(
                    "Deploy script timed out after {}s for {} @ {}",
                    self.script_timeout.as_secs(),
                    project,
                    ref_name
                );
                (-1, String::new(), format!("Script timed out after {}s", self.script_timeout.as_secs()))
            }
            ScriptResult::Error(e) => {
                (-1, String::new(), e)
            }
        };

        DeployResult {
            exit_code,
            stdout: truncate_output(&stdout_raw),
            stderr: truncate_output(&stderr_raw),
            commit,
            duration: start.elapsed(),
            cancelled: false,
        }
    }

    /// Spawn and run the deploy script, collecting stdout/stderr concurrently.
    async fn run_script(
        &self,
        script_path: &std::path::Path,
        ref_name: &str,
        cancel_token: CancellationToken,
    ) -> ScriptResult {
        let script_abs = script_path.canonicalize().unwrap_or_else(|_| script_path.to_path_buf());
        let mut child = match Command::new("bash")
            .arg(&script_abs)
            .env("GIT_BRANCH", ref_name)
            .env("VERSION", ref_name)
            .env("HARBOR_REGISTRY", &self.harbor.registry)
            .env("HARBOR_PROJECT", &self.harbor.project)
            .env("HARBOR_USER", &self.harbor.user)
            .env("HARBOR_PASSWORD", &self.harbor.password)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return ScriptResult::Error(format!("Failed to spawn script: {}", e)),
        };

        let pid = child.id();

        // Take stdout/stderr pipes
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        // Read stdout and stderr concurrently with wait
        let read_stdout = tokio::spawn(read_stdout_pipe(stdout_pipe));
        let read_stderr = tokio::spawn(read_stderr_pipe(stderr_pipe));

        let timeout_fut = tokio::time::sleep(self.script_timeout);
        let cancel_fut = cancel_token.cancelled();

        // Wait for child in the select (not as a spawned task, to avoid move issues)
        tokio::select! {
            _ = cancel_fut => {
                kill_child(pid).await;
                let _ = tokio::time::timeout(Duration::from_secs(2), read_stdout).await;
                let _ = tokio::time::timeout(Duration::from_secs(2), read_stderr).await;
                let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                ScriptResult::Cancelled
            }
            _ = timeout_fut => {
                kill_child(pid).await;
                let _ = tokio::time::timeout(Duration::from_secs(2), read_stdout).await;
                let _ = tokio::time::timeout(Duration::from_secs(2), read_stderr).await;
                let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                ScriptResult::Timeout
            }
            status_result = child.wait() => {
                let stdout = read_stdout.await.unwrap_or_default();
                let stderr = read_stderr.await.unwrap_or_default();
                let exit_code = match status_result {
                    Ok(status) => status.code().unwrap_or(-1),
                    Err(_) => -1,
                };
                ScriptResult::Completed {
                    code: exit_code,
                    stdout,
                    stderr,
                }
            }
        }
    }
}

enum ScriptResult {
    Completed { code: i32, stdout: String, stderr: String },
    Cancelled,
    Timeout,
    Error(String),
}

/// Read all data from an optional stdout pipe into a String.
async fn read_stdout_pipe(pipe: Option<tokio::process::ChildStdout>) -> String {
    use tokio::io::AsyncReadExt;
    let mut pipe = match pipe {
        Some(p) => p,
        None => return String::new(),
    };
    let mut buf = Vec::new();
    match pipe.read_to_end(&mut buf).await {
        Ok(_) => String::from_utf8_lossy(&buf).to_string(),
        Err(_) => String::new(),
    }
}

/// Read all data from an optional stderr pipe into a String.
async fn read_stderr_pipe(pipe: Option<tokio::process::ChildStderr>) -> String {
    use tokio::io::AsyncReadExt;
    let mut pipe = match pipe {
        Some(p) => p,
        None => return String::new(),
    };
    let mut buf = Vec::new();
    match pipe.read_to_end(&mut buf).await {
        Ok(_) => String::from_utf8_lossy(&buf).to_string(),
        Err(_) => String::new(),
    }
}

/// Kill a child process and its process group.
async fn kill_child(pid: Option<u32>) {
    if let Some(pid) = pid {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .output()
            .await;
        let _ = Command::new("kill")
            .arg("-9")
            .arg(format!("-{}", pid))
            .output()
            .await;
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
        assert!(result.len() <= OUTPUT_MAX + 100);
        assert!(result.contains("["));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_output_max_constant() {
        assert_eq!(HEAD_SIZE + TAIL_SIZE, OUTPUT_MAX);
    }
}
