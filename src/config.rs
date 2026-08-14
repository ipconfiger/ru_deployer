//! Configuration loading: TOML file + environment variable overrides.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level application configuration.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub gitlab: GitLabConfig,
    pub deploy: DeployConfig,
    pub filter: FilterConfig,
    pub notify: NotifyConfig,
    #[serde(default)]
    pub harbor: HarborConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitLabConfig {
    #[serde(default = "default_gitlab_url")]
    pub url: String,
    pub token: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployConfig {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_scripts_dir")]
    pub scripts_dir: PathBuf,
    #[serde(default = "default_src_dir")]
    pub src_dir: PathBuf,
    #[serde(default = "default_script_timeout")]
    pub script_timeout_secs: u64,
    pub project: Option<String>,
    pub branch: Option<String>,
    #[serde(default)]
    pub release_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub file: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct NotifyConfig {
    #[serde(default = "default_msg_platform_url")]
    pub msg_platform_url: String,
    #[serde(default = "default_true")]
    pub notify_author: bool,
    #[serde(default)]
    pub cc: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HarborConfig {
    #[serde(default)]
    pub registry: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_output")]
    pub output: String,
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: PathBuf,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            output: "stdout".into(),
            file: None,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./data/deploy.db"),
        }
    }
}

// --- Default value functions ---
fn default_gitlab_url() -> String {
    "https://gitlab.do.top".into()
}
fn default_poll_interval() -> u64 {
    10
}
fn default_mode() -> String {
    "multi".into()
}
fn default_scripts_dir() -> PathBuf {
    PathBuf::from("./scripts")
}
fn default_src_dir() -> PathBuf {
    PathBuf::from("./src")
}
fn default_script_timeout() -> u64 {
    1800
}
fn default_msg_platform_url() -> String {
    "http://172.16.42.213:8081".into()
}
fn default_log_level() -> String {
    "info".into()
}
fn default_output() -> String {
    "stdout".into()
}
fn default_db_path() -> PathBuf {
    PathBuf::from("./data/deploy.db")
}
fn default_true() -> bool {
    true
}

/// Load config from TOML file, then apply environment variable overrides.
///
/// Environment variables follow `RU_DEPLOYER_<SECTION>_<KEY>` format.
pub fn load(config_path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    let mut config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

    // Environment variable overrides
    if let Ok(v) = std::env::var("RU_DEPLOYER_GITLAB_URL") {
        config.gitlab.url = v;
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_GITLAB_TOKEN") {
        config.gitlab.token = v;
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_GITLAB_POLL_INTERVAL_SECS") {
        if let Ok(n) = v.parse() {
            config.gitlab.poll_interval_secs = n;
        }
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_MODE") {
        config.deploy.mode = v;
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_SCRIPTS_DIR") {
        config.deploy.scripts_dir = PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_SRC_DIR") {
        config.deploy.src_dir = PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_SCRIPT_TIMEOUT_SECS") {
        if let Ok(n) = v.parse() {
            config.deploy.script_timeout_secs = n;
        }
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_PROJECT") {
        config.deploy.project = Some(v);
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_BRANCH") {
        config.deploy.branch = Some(v);
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_DEPLOY_RELEASE_PREFIX") {
        config.deploy.release_prefix = Some(v);
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_FILTER_FILE") {
        config.filter.file = Some(PathBuf::from(v));
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_NOTIFY_MSG_PLATFORM_URL") {
        config.notify.msg_platform_url = v;
    }
    // Harbor: RU_DEPLOYER_HARBOR_* 优先；plain HARBOR_PASSWORD 兜底
    // （config.toml / docs/design.md 声明 plain HARBOR_PASSWORD 可用；
    //  原依赖 push_harbor.sh 写死密码兜底，T1 删除兜底后必须在此读取，
    //  否则 .env/EnvironmentFile 中的 HARBOR_PASSWORD 会静默失效）
    if let Ok(v) = std::env::var("RU_DEPLOYER_HARBOR_REGISTRY") {
        config.harbor.registry = v;
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_HARBOR_PROJECT") {
        config.harbor.project = v;
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_HARBOR_USER") {
        config.harbor.user = v;
    }
    if let Ok(v) = std::env::var("RU_DEPLOYER_HARBOR_PASSWORD") {
        config.harbor.password = v;
    } else if let Ok(v) = std::env::var("HARBOR_PASSWORD") {
        config.harbor.password = v;
    }

    // Normalize release_prefix: empty/whitespace → None (no filtering)
    if let Some(p) = config.deploy.release_prefix.as_deref() {
        let trimmed = p.trim();
        config.deploy.release_prefix = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests that manipulate process-level env vars
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_load_minimal_config() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("RU_DEPLOYER_GITLAB_TOKEN");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let toml_content = r#"
[gitlab]
token = "test-token"

[deploy]
scripts_dir = "./test-scripts"

[filter]

[notify]
"#;
        std::fs::write(&config_path, toml_content).unwrap();
        let config = load(&config_path).unwrap();
        assert_eq!(config.gitlab.token, "test-token");
        assert_eq!(config.deploy.scripts_dir, PathBuf::from("./test-scripts"));
        assert_eq!(config.deploy.mode, "multi");
        assert_eq!(config.gitlab.url, "https://gitlab.do.top");
    }

    #[test]
    fn test_env_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("RU_DEPLOYER_GITLAB_TOKEN");
        std::env::remove_var("RU_DEPLOYER_DEPLOY_MODE");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let toml_content = r#"
[gitlab]
token = "file-token"

[deploy]

[filter]

[notify]
"#;
        std::fs::write(&config_path, toml_content).unwrap();

        std::env::set_var("RU_DEPLOYER_GITLAB_TOKEN", "env-token");
        std::env::set_var("RU_DEPLOYER_DEPLOY_MODE", "events");
        let config = load(&config_path).unwrap();
        assert_eq!(config.gitlab.token, "env-token");
        assert_eq!(config.deploy.mode, "events");

        std::env::remove_var("RU_DEPLOYER_GITLAB_TOKEN");
        std::env::remove_var("RU_DEPLOYER_DEPLOY_MODE");
    }

    #[test]
    fn test_release_prefix() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("RU_DEPLOYER_GITLAB_TOKEN");
        std::env::remove_var("RU_DEPLOYER_DEPLOY_RELEASE_PREFIX");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let toml_content = r#"
[gitlab]
token = "test-token"

[deploy]
release_prefix = "gpu-"

[filter]

[notify]
"#;
        std::fs::write(&config_path, toml_content).unwrap();
        let config = load(&config_path).unwrap();
        assert_eq!(config.deploy.release_prefix.as_deref(), Some("gpu-"));

        // Env override with pure whitespace should normalize to None (no filtering)
        std::env::set_var("RU_DEPLOYER_DEPLOY_RELEASE_PREFIX", "  ");
        let config = load(&config_path).unwrap();
        assert_eq!(config.deploy.release_prefix, None);

        std::env::remove_var("RU_DEPLOYER_DEPLOY_RELEASE_PREFIX");
    }

    #[test]
    fn test_harbor_env_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("RU_DEPLOYER_GITLAB_TOKEN");
        std::env::remove_var("RU_DEPLOYER_HARBOR_REGISTRY");
        std::env::remove_var("RU_DEPLOYER_HARBOR_PROJECT");
        std::env::remove_var("RU_DEPLOYER_HARBOR_USER");
        std::env::remove_var("RU_DEPLOYER_HARBOR_PASSWORD");
        std::env::remove_var("HARBOR_PASSWORD");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let toml_content = r#"
[gitlab]
token = "test-token"

[deploy]

[filter]

[notify]

[harbor]
registry = "reg.example.com:5000"
project = "proj"
user = "robot$proj+bot"
password = ""
"#;
        std::fs::write(&config_path, toml_content).unwrap();

        // RU_DEPLOYER_HARBOR_* 优先于 plain HARBOR_PASSWORD
        std::env::set_var("RU_DEPLOYER_HARBOR_REGISTRY", "env-reg:5000");
        std::env::set_var("RU_DEPLOYER_HARBOR_PROJECT", "env-proj");
        std::env::set_var("RU_DEPLOYER_HARBOR_USER", "env-user");
        std::env::set_var("RU_DEPLOYER_HARBOR_PASSWORD", "env-pass");
        std::env::set_var("HARBOR_PASSWORD", "plain-pass");
        let config = load(&config_path).unwrap();
        assert_eq!(config.harbor.registry, "env-reg:5000");
        assert_eq!(config.harbor.project, "env-proj");
        assert_eq!(config.harbor.user, "env-user");
        assert_eq!(config.harbor.password, "env-pass");

        // 无 RU_DEPLOYER_ 前缀时 plain HARBOR_PASSWORD 兜底
        std::env::remove_var("RU_DEPLOYER_HARBOR_PASSWORD");
        let config = load(&config_path).unwrap();
        assert_eq!(config.harbor.password, "plain-pass");

        // 全部清空则回落到 TOML 文件值
        std::env::remove_var("RU_DEPLOYER_HARBOR_REGISTRY");
        std::env::remove_var("RU_DEPLOYER_HARBOR_PROJECT");
        std::env::remove_var("RU_DEPLOYER_HARBOR_USER");
        std::env::remove_var("HARBOR_PASSWORD");
        let config = load(&config_path).unwrap();
        assert_eq!(config.harbor.registry, "reg.example.com:5000");
        assert_eq!(config.harbor.project, "proj");
        assert_eq!(config.harbor.user, "robot$proj+bot");
        assert_eq!(config.harbor.password, "");
    }
}
