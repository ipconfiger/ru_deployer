//! Git operations via system `git` CLI.
//!
//! Provides clone, fetch, and checkout for deploying projects to
//! `src/<short_name>/<branch>/`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{debug, info};

/// Handles git clone/fetch operations for deployment source directories.
#[derive(Debug, Clone)]
pub struct GitRepo {
    src_dir: PathBuf,
}

impl GitRepo {
    /// Create a new GitRepo manager rooted at `src_dir`.
    pub fn new(src_dir: PathBuf) -> Self {
        Self { src_dir }
    }

    /// Ensure the specified project+branch is checked out at the latest commit.
    ///
    /// Directory structure: `src_dir/<short_name>/<branch>/`
    /// where `short_name` is the last segment of the project path
    /// (e.g., "dev-team/api" → "api").
    ///
    /// Returns the absolute path to the local repository.
    pub async fn ensure(
        &self,
        gitlab_host: &str,
        token: &str,
        project: &str,
        branch: &str,
    ) -> Result<PathBuf> {
        // Safety check: reject branch names with ".." to prevent path traversal
        if branch.contains("..") {
            anyhow::bail!("Invalid branch name (contains '..'): {}", branch);
        }

        let short_name = project.rsplit('/').next().unwrap_or(project);
        let target_dir = self.src_dir.join(short_name).join(branch);

        // Build auth URL
        let repo_url = if token.is_empty() {
            format!("http://{}/{}.git", gitlab_host, project)
        } else {
            format!("http://oauth2:{}@{}/{}.git", token, gitlab_host, project)
        };

        if target_dir.join(".git").exists() {
            debug!("Repository exists at {}, fetching updates", target_dir.display());
            self.fetch_and_reset(&target_dir, branch).await?;
        } else {
            info!("Cloning {} @ {} into {}", project, branch, target_dir.display());
            self.clone(&target_dir, &repo_url, branch).await?;
        }

        Ok(target_dir)
    }

    /// Get the current HEAD commit SHA of a local repository.
    pub async fn current_commit(&self, repo_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to run git rev-parse in {}",
                    repo_path.display()
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git rev-parse failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get the short HEAD commit SHA (first 8 chars).
    pub async fn short_commit(&self, repo_path: &Path) -> Result<String> {
        let full = self.current_commit(repo_path).await?;
        Ok(full.chars().take(8).collect())
    }

    // --- Private helpers ---

    async fn clone(&self, target: &Path, repo_url: &str, branch: &str) -> Result<()> {
        std::fs::create_dir_all(target)
            .with_context(|| format!("Failed to create directory: {}", target.display()))?;

        let output = Command::new("git")
            .args([
                "clone",
                "--depth", "1",
                "--branch", branch,
                repo_url,
            ])
            .arg(target.as_os_str())
            .output()
            .await
            .context("Failed to run git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git clone failed: {}", stderr);
        }

        Ok(())
    }

    async fn fetch_and_reset(&self, repo_path: &Path, branch: &str) -> Result<()> {
        // git fetch origin <branch> --depth 1
        let output = Command::new("git")
            .args(["fetch", "origin", branch, "--depth", "1"])
            .current_dir(repo_path)
            .output()
            .await
            .context("Failed to run git fetch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git fetch failed: {}", stderr);
        }

        // git checkout <branch>
        let output = Command::new("git")
            .args(["checkout", branch])
            .current_dir(repo_path)
            .output()
            .await
            .context("Failed to run git checkout")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git checkout failed: {}", stderr);
        }

        // git reset --hard origin/<branch>
        let remote_ref = format!("origin/{}", branch);
        let output = Command::new("git")
            .args(["reset", "--hard", &remote_ref])
            .current_dir(repo_path)
            .output()
            .await
            .context("Failed to run git reset")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git reset failed: {}", stderr);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branch_traversal_rejected() {
        let repo = GitRepo::new(PathBuf::from("/tmp/test"));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(repo.ensure(
            "gitlab.example.com",
            "token",
            "dev-team/api",
            "../etc",
        ));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".."));
    }

    #[test]
    fn test_short_name_extraction() {
        // Verify the path logic by checking short name extraction
        assert_eq!("api", "dev-team/api".rsplit('/').next().unwrap());
        assert_eq!("project", "project".rsplit('/').next().unwrap());
        assert_eq!("sub", "group/sub".rsplit('/').next().unwrap());
    }
}
