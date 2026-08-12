//! SQLite deployment history storage.
//!
//! Persists every deployment attempt with output truncation for diagnostics.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

/// Database handle for deployment history.
#[derive(Debug, Clone)]
pub struct DeploymentDb {
    pool: SqlitePool,
}

/// A single deployment record.
#[derive(Debug, Clone)]
pub struct DeploymentRecord {
    pub project: String,
    pub branch: String,
    pub commit_sha: String,
    pub author_name: String,
    pub author_email: String,
    pub event_id: u64,
    pub exit_code: i32,
    pub status: String,       // "success" or "failed"
    pub stdout_tail: String,  // head 2KB + tail 8KB
    pub stderr_tail: String,  // head 2KB + tail 8KB
    pub duration_ms: i64,
}

/// Deployment statistics for a project.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DeployStats {
    pub total: u64,
    pub success: u64,
    pub failed: u64,
}

impl DeploymentDb {
    /// Open or create the SQLite database file and run migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }

        let db_url = format!("sqlite:{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .with_context(|| format!("Failed to open database: {}", path.display()))?;

        // Enable WAL mode for better concurrent performance
        sqlx::query("PRAGMA journal_mode=WAL;")
            .execute(&pool)
            .await
            .context("Failed to enable WAL mode")?;

        // Run migration
        Self::migrate(&pool).await?;

        tracing::info!("Database opened: {}", path.display());
        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS deployments (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                project      TEXT    NOT NULL,
                branch       TEXT    NOT NULL,
                commit_sha   TEXT    NOT NULL,
                author_name  TEXT    NOT NULL,
                author_email TEXT    NOT NULL DEFAULT '',
                event_id     INTEGER NOT NULL,
                exit_code    INTEGER NOT NULL,
                status       TEXT    NOT NULL CHECK (status IN ('success', 'failed')),
                stdout_tail  TEXT    NOT NULL DEFAULT '',
                stderr_tail  TEXT    NOT NULL DEFAULT '',
                duration_ms  INTEGER NOT NULL,
                created_at   TEXT    NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )
        .execute(pool)
        .await
        .context("Failed to create deployments table")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_deployments_project ON deployments(project);",
        )
        .execute(pool)
        .await
        .context("Failed to create index idx_deployments_project")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_deployments_project_branch ON deployments(project, branch);",
        )
        .execute(pool)
        .await
        .context("Failed to create index idx_deployments_project_branch")?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_deployments_created_at ON deployments(created_at);",
        )
        .execute(pool)
        .await
        .context("Failed to create index idx_deployments_created_at")?;

        Ok(())
    }

    /// Insert a deployment record and return its ID.
    pub async fn insert(&self, record: &DeploymentRecord) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO deployments
                (project, branch, commit_sha, author_name, author_email, event_id,
                 exit_code, status, stdout_tail, stderr_tail, duration_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11);
            "#,
        )
        .bind(&record.project)
        .bind(&record.branch)
        .bind(&record.commit_sha)
        .bind(&record.author_name)
        .bind(&record.author_email)
        .bind(record.event_id as i64)
        .bind(record.exit_code)
        .bind(&record.status)
        .bind(&record.stdout_tail)
        .bind(&record.stderr_tail)
        .bind(record.duration_ms)
        .execute(&self.pool)
        .await
        .context("Failed to insert deployment record")?;

        Ok(result.last_insert_rowid())
    }

    /// Fetch the most recent N deployments for a project.
    #[allow(dead_code)]
    pub async fn recent(&self, project: &str, limit: u32) -> Result<Vec<DeploymentRecord>> {
        let rows = sqlx::query_as::<_, DeploymentRow>(
            r#"
            SELECT project, branch, commit_sha, author_name, author_email,
                   event_id, exit_code, status, stdout_tail, stderr_tail, duration_ms
            FROM deployments
            WHERE project = ?1
            ORDER BY id DESC
            LIMIT ?2;
            "#,
        )
        .bind(project)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("Failed to query recent deployments")?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Fetch the most recent N deployments for a project + branch.
    #[allow(dead_code)]
    pub async fn recent_by_branch(
        &self,
        project: &str,
        branch: &str,
        limit: u32,
    ) -> Result<Vec<DeploymentRecord>> {
        let rows = sqlx::query_as::<_, DeploymentRow>(
            r#"
            SELECT project, branch, commit_sha, author_name, author_email,
                   event_id, exit_code, status, stdout_tail, stderr_tail, duration_ms
            FROM deployments
            WHERE project = ?1 AND branch = ?2
            ORDER BY id DESC
            LIMIT ?3;
            "#,
        )
        .bind(project)
        .bind(branch)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("Failed to query recent deployments by branch")?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get deployment statistics for a project in the last N days.
    #[allow(dead_code)]
    pub async fn stats(&self, project: &str, days: u32) -> Result<DeployStats> {
        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM deployments
            WHERE project = ?1 AND created_at >= datetime('now', ?2);
            "#,
        )
        .bind(project)
        .bind(format!("-{} days", days))
        .fetch_one(&self.pool)
        .await
        .context("Failed to query deployment stats")?;

        let success: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM deployments
            WHERE project = ?1 AND status = 'success' AND created_at >= datetime('now', ?2);
            "#,
        )
        .bind(project)
        .bind(format!("-{} days", days))
        .fetch_one(&self.pool)
        .await
        .context("Failed to query deployment success stats")?;

        Ok(DeployStats {
            total: total.0 as u64,
            success: success.0 as u64,
            failed: total.0 as u64 - success.0 as u64,
        })
    }
}

/// Database row type for sqlx::query_as.
#[derive(Debug, sqlx::FromRow)]
struct DeploymentRow {
    project: String,
    branch: String,
    commit_sha: String,
    author_name: String,
    author_email: String,
    event_id: i64,
    exit_code: i32,
    status: String,
    stdout_tail: String,
    stderr_tail: String,
    duration_ms: i64,
}

impl From<DeploymentRow> for DeploymentRecord {
    fn from(row: DeploymentRow) -> Self {
        Self {
            project: row.project,
            branch: row.branch,
            commit_sha: row.commit_sha,
            author_name: row.author_name,
            author_email: row.author_email,
            event_id: row.event_id as u64,
            exit_code: row.exit_code,
            status: row.status,
            stdout_tail: row.stdout_tail,
            stderr_tail: row.stderr_tail,
            duration_ms: row.duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_db() -> DeploymentDb {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        DeploymentDb::open(&path).await.unwrap()
    }

    #[tokio::test]
    async fn test_insert_and_recent() {
        let db = create_test_db().await;

        let record = DeploymentRecord {
            project: "dev-team/api".into(),
            branch: "main".into(),
            commit_sha: "abcdef1234567890".into(),
            author_name: "testuser".into(),
            author_email: "test@example.com".into(),
            event_id: 12345,
            exit_code: 0,
            status: "success".into(),
            stdout_tail: "build output".into(),
            stderr_tail: "".into(),
            duration_ms: 5000,
        };

        let id = db.insert(&record).await.unwrap();
        assert!(id > 0);

        let recent = db.recent("dev-team/api", 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].project, "dev-team/api");
        assert_eq!(recent[0].status, "success");
    }

    #[tokio::test]
    async fn test_recent_by_branch() {
        let db = create_test_db().await;

        let r1 = DeploymentRecord {
            project: "dev-team/api".into(),
            branch: "main".into(),
            commit_sha: "aaa".into(),
            author_name: "u1".into(),
            author_email: "".into(),
            event_id: 1,
            exit_code: 0,
            status: "success".into(),
            stdout_tail: "".into(),
            stderr_tail: "".into(),
            duration_ms: 1000,
        };

        let r2 = DeploymentRecord {
            project: "dev-team/api".into(),
            branch: "develop".into(),
            commit_sha: "bbb".into(),
            author_name: "u2".into(),
            author_email: "".into(),
            event_id: 2,
            exit_code: 1,
            status: "failed".into(),
            stdout_tail: "".into(),
            stderr_tail: "".into(),
            duration_ms: 2000,
        };

        db.insert(&r1).await.unwrap();
        db.insert(&r2).await.unwrap();

        let main_deploys = db.recent_by_branch("dev-team/api", "main", 10).await.unwrap();
        assert_eq!(main_deploys.len(), 1);
        assert_eq!(main_deploys[0].branch, "main");

        let dev_deploys = db.recent_by_branch("dev-team/api", "develop", 10).await.unwrap();
        assert_eq!(dev_deploys.len(), 1);
        assert_eq!(dev_deploys[0].status, "failed");
    }

    #[tokio::test]
    async fn test_stats() {
        let db = create_test_db().await;

        for i in 0..5 {
            let record = DeploymentRecord {
                project: "dev-team/api".into(),
                branch: "main".into(),
                commit_sha: format!("sha{}", i),
                author_name: "u".into(),
                author_email: "".into(),
                event_id: i as u64,
                exit_code: if i < 3 { 0 } else { 1 },
                status: if i < 3 { "success".into() } else { "failed".into() },
                stdout_tail: "".into(),
                stderr_tail: "".into(),
                duration_ms: 1000,
            };
            db.insert(&record).await.unwrap();
        }

        let stats = db.stats("dev-team/api", 365).await.unwrap();
        assert_eq!(stats.total, 5);
        assert_eq!(stats.success, 3);
        assert_eq!(stats.failed, 2);
    }
}
