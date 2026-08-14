//! One-shot manual deployment: pull the latest code for a single service and
//! run its deploy script, then exit (no polling loop).

use crate::config::Config;
use crate::deploy::Deployer;
use crate::filter::Filter;
use crate::types::PushEvent;
use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Run a single deployment for `service` (short name like "api", or full path
/// like "dev-team/api"). The branch is taken from the filter config unless
/// `branch_override` is provided.
pub async fn run(cfg: &Config, service: &str, branch_override: Option<&str>) -> Result<()> {
    let filter = Filter::load(cfg.filter.file.as_deref())?;

    // Match by short name (last path segment) or full project path.
    let repo = filter.repos.iter().find(|r| {
        r.project == service || r.project.rsplit('/').next().unwrap_or(&r.project) == service
    });
    let Some(repo) = repo else {
        let known: Vec<&str> = filter.project_paths().collect();
        anyhow::bail!(
            "service '{}' not found in filter; configured projects: {}",
            service,
            if known.is_empty() { "(none)".to_string() } else { known.join(", ") }
        );
    };
    let project = repo.project.clone();

    // Branch: explicit --branch wins. Otherwise use the single configured
    // branch; when multiple branches are configured, --branch is required.
    let branch = match branch_override {
        Some(b) => b.to_string(),
        None => match repo.branches.len() {
            0 => anyhow::bail!("project '{}' has no branch configured", project),
            1 => repo.branches[0].clone(),
            _ => anyhow::bail!(
                "project '{}' has {} branches configured ({}); specify one with --branch",
                project,
                repo.branches.len(),
                repo.branches.join(", ")
            ),
        },
    };

    info!("one-shot deploy: {} @ {}", project, branch);

    let event = PushEvent {
        event_id: 0,
        project_id: 0,
        project: project.clone(),
        branch: branch.clone(),
        commit: String::new(),
        author_name: "manual".into(),
        author_id: 0,
        created_at: None,
    };

    let deployer = Deployer::new(
        cfg.deploy.src_dir.clone(),
        cfg.deploy.scripts_dir.clone(),
        cfg.deploy.script_timeout_secs,
        cfg.harbor.clone(),
    );

    let cancel = CancellationToken::new();
    let result = deployer
        .deploy(&event, &cfg.gitlab.url, &cfg.gitlab.token, cancel)
        .await;

    if result.cancelled {
        anyhow::bail!("one-shot deploy cancelled");
    }
    if result.exit_code != 0 {
        anyhow::bail!(
            "one-shot deploy failed for {} @ {} (exit={})",
            project, branch, result.exit_code
        );
    }

    info!(
        "one-shot deploy succeeded: {} @ {} (commit {}, {}s)",
        project, branch, result.commit, result.duration.as_secs()
    );

    Ok(())
}
