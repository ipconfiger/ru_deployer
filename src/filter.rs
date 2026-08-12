//! Project/branch filter for push events.
//!
//! Loads from a TOML file and determines whether a given (project, branch)
//! combination should trigger deployment.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Filter configuration loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
struct FilterToml {
    #[serde(default)]
    repos: Vec<RepoRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoRule {
    pub project: String,
    #[serde(default)]
    pub branches: Vec<String>,
}

/// Runtime filter that matches push events against configured rules.
#[derive(Debug, Clone)]
pub struct Filter {
    pub repos: Vec<RepoRule>,
}

impl Filter {
    /// Load filter from a TOML file. Returns an empty filter (matches all) if
    /// the file does not exist, or if `path` is None.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(p) => p,
            None => return Ok(Self { repos: vec![] }),
        };

        if !path.exists() {
            tracing::warn!("Filter file not found: {}, treating as match-all", path.display());
            return Ok(Self { repos: vec![] });
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read filter file: {}", path.display()))?;

        let filter: FilterToml = toml::from_str(&content)
            .with_context(|| format!("Failed to parse filter file: {}", path.display()))?;

        tracing::info!("Loaded filter with {} repo(s)", filter.repos.len());
        Ok(Self {
            repos: filter.repos,
        })
    }

    /// Returns true if no repos are configured (match-all mode).
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// Returns an iterator over configured project paths (for startup validation).
    pub fn project_paths(&self) -> impl Iterator<Item = &str> {
        self.repos.iter().map(|r| r.project.as_str())
    }

    /// Check whether a push event for the given project and branch should be
    /// handled. Branch is automatically stripped of `refs/heads/` prefix before
    /// matching.
    ///
    /// Matching rules (aligned with Python listen_push.py):
    /// 1. Empty filter → match all
    /// 2. Project not in repos → reject
    /// 3. `branches` is empty → match all branches for this project
    /// 4. `branches` non-empty → exact branch name match
    pub fn matches(&self, project: &str, branch: &str) -> bool {
        if self.is_empty() {
            return true;
        }

        // Strip refs/heads/ prefix if present
        let branch = branch.trim_start_matches("refs/heads/");

        for repo in &self.repos {
            if repo.project == project {
                if repo.branches.is_empty() {
                    return true;
                }
                return repo.branches.iter().any(|b| b == branch);
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_filter_matches_all() {
        let filter = Filter { repos: vec![] };
        assert!(filter.is_empty());
        assert!(filter.matches("dev-team/api", "main"));
        assert!(filter.matches("any/project", "any-branch"));
    }

    #[test]
    fn test_exact_branch_match() {
        let filter = Filter {
            repos: vec![RepoRule {
                project: "dev-team/api".into(),
                branches: vec!["main".into(), "develop".into()],
            }],
        };
        assert!(filter.matches("dev-team/api", "main"));
        assert!(filter.matches("dev-team/api", "develop"));
        assert!(!filter.matches("dev-team/api", "feature/x"));
        assert!(!filter.matches("dev-team/other", "main"));
    }

    #[test]
    fn test_refs_heads_prefix_stripping() {
        let filter = Filter {
            repos: vec![RepoRule {
                project: "dev-team/api".into(),
                branches: vec!["main".into()],
            }],
        };
        assert!(filter.matches("dev-team/api", "refs/heads/main"));
        assert!(filter.matches("dev-team/api", "main"));
        assert!(!filter.matches("dev-team/api", "refs/heads/develop"));
    }

    #[test]
    fn test_empty_branches_means_all() {
        let filter = Filter {
            repos: vec![RepoRule {
                project: "dev-team/api".into(),
                branches: vec![],
            }],
        };
        assert!(filter.matches("dev-team/api", "main"));
        assert!(filter.matches("dev-team/api", "feature/x"));
        assert!(filter.matches("dev-team/api", "refs/heads/anything"));
    }

    #[test]
    fn test_project_not_in_filter() {
        let filter = Filter {
            repos: vec![RepoRule {
                project: "dev-team/api".into(),
                branches: vec!["main".into()],
            }],
        };
        assert!(!filter.matches("dev-team/other", "main"));
    }

    #[test]
    fn test_load_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filter.toml");
        let content = r#"
[[repos]]
project = "dev-team/api"
branches = ["main", "mysql_db"]

[[repos]]
project = "dev-team/flint"
branches = []
"#;
        std::fs::write(&path, content).unwrap();

        let filter = Filter::load(Some(&path)).unwrap();
        assert_eq!(filter.repos.len(), 2);
        assert!(filter.matches("dev-team/api", "main"));
        assert!(filter.matches("dev-team/flint", "any-branch"));
        assert!(!filter.matches("unknown/project", "main"));
    }

    #[test]
    fn test_load_none_file() {
        let filter = Filter::load(None).unwrap();
        assert!(filter.is_empty());
    }

    #[test]
    fn test_load_missing_file() {
        let filter = Filter::load(Some(Path::new("/nonexistent/filter.toml"))).unwrap();
        assert!(filter.is_empty());
    }
}
