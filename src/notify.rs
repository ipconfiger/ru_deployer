//! Email notification via message platform API.
//!
//! Sends HTML emails for deployment success/failure.

use crate::deploy::DeployResult;
use crate::types::{PushEvent, ReleaseEvent};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use tracing::{error, info};

/// Email notifier backed by the message platform API.
#[derive(Debug, Clone)]
pub struct Notifier {
    api_url: String,
    client: Client,
}

#[derive(Serialize)]
struct SendRawRequest<'a> {
    to: &'a [String],
    subject: String,
    content_type: &'a str,
    content: String,
}

impl Notifier {
    pub fn new(api_url: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("Failed to build HTTP client for notifications")?;

        Ok(Self { api_url, client })
    }

    /// Send a deployment result notification.
    pub async fn notify(
        &self,
        event: &PushEvent,
        result: &DeployResult,
        author_email: &str,
    ) {
        if author_email.is_empty() {
            info!("No author email available, skipping notification");
            return;
        }

        let short_commit: String = event.commit.chars().take(8).collect();
        let (status_icon, status_text) = if result.exit_code == 0 {
            ("✅", "成功")
        } else {
            ("❌", "失败")
        };

        let subject = format!(
            "[{}] {}@{} 部署{}",
            status_icon,
            event.project.rsplit('/').next().unwrap_or(&event.project),
            event.branch,
            status_text
        );

        let duration_secs = result.duration.as_secs();
        let created_at = event
            .created_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".into());

        let content = if result.exit_code == 0 {
            format!(
                r#"<h2>{} 部署成功</h2>
<p><strong>项目:</strong> {}</p>
<p><strong>分支:</strong> {}</p>
<p><strong>commit:</strong> {}</p>
<p><strong>耗时:</strong> {}s</p>
<p><strong>时间:</strong> {}</p>"#,
                status_icon,
                html_escape(&event.project),
                html_escape(&event.branch),
                html_escape(&short_commit),
                duration_secs,
                html_escape(&created_at),
            )
        } else {
            let error_log = format!(
                "{}\n{}",
                &result.stderr,
                &result.stdout
            );
            format!(
                r#"<h2>{} 部署失败</h2>
<p><strong>项目:</strong> {}</p>
<p><strong>分支:</strong> {}</p>
<p><strong>commit:</strong> {}</p>
<p><strong>退出码:</strong> {}</p>
<p><strong>耗时:</strong> {}s</p>
<p><strong>时间:</strong> {}</p>
<hr>
<h3>错误日志:</h3>
<pre style="background:#1e1e1e;color:#d4d4d4;padding:12px;border-radius:6px;overflow-x:auto;font-size:12px;line-height:1.5">{}</pre>"#,
                status_icon,
                html_escape(&event.project),
                html_escape(&event.branch),
                html_escape(&short_commit),
                result.exit_code,
                duration_secs,
                html_escape(&created_at),
                html_escape(&error_log),
            )
        };

        let req = SendRawRequest {
            to: &[author_email.to_string()],
            subject,
            content_type: "html",
            content,
        };

        let url = format!("{}/api/v1/mail/send_raw", self.api_url.trim_end_matches('/'));

        match self.client.post(&url).json(&req).send().await {
            Ok(resp) => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                if body.get("code").and_then(|c| c.as_i64()) == Some(0) {
                    let msg_id = body["data"]["message_id"].as_str().unwrap_or("?");
                    info!("Email sent to {} (message_id={})", author_email, msg_id);
                } else {
                    let msg = body.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
                    error!("Email API returned error: {}", msg);
                }
            }
            Err(e) => {
                error!("Failed to send email notification: {}", e);
            }
        }
    }

    /// Send a release build result notification.
    pub async fn notify_release(
        &self,
        event: &ReleaseEvent,
        result: &DeployResult,
        author_email: &str,
    ) {
        if author_email.is_empty() {
            info!("No author email available, skipping release notification");
            return;
        }

        let (status_icon, status_text) = if result.exit_code == 0 {
            ("✅", "成功")
        } else {
            ("❌", "失败")
        };

        let subject = format!(
            "[{}] {} Release {} 构建{}",
            status_icon,
            event.project.rsplit('/').next().unwrap_or(&event.project),
            event.tag_name,
            status_text
        );

        let content = format!(
            r#"<h2>{} Release 构建{}</h2>
<p><strong>项目:</strong> {}</p>
<p><strong>版本:</strong> {}</p>
<p><strong>退出码:</strong> {}</p>
<p><strong>耗时:</strong> {}s</p>"#,
            status_icon,
            status_text,
            html_escape(&event.project),
            html_escape(&event.tag_name),
            result.exit_code,
            result.duration.as_secs(),
        );

        let req = SendRawRequest {
            to: &[author_email.to_string()],
            subject,
            content_type: "html",
            content,
        };

        let url = format!("{}/api/v1/mail/send_raw", self.api_url.trim_end_matches('/'));

        match self.client.post(&url).json(&req).send().await {
            Ok(resp) => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                if body.get("code").and_then(|c| c.as_i64()) == Some(0) {
                    let msg_id = body["data"]["message_id"].as_str().unwrap_or("?");
                    info!("Release email sent to {} (message_id={})", author_email, msg_id);
                } else {
                    let msg = body.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
                    error!("Release email API returned error: {}", msg);
                }
            }
            Err(e) => {
                error!("Failed to send release email notification: {}", e);
            }
        }
    }
}

/// Minimal HTML entity escaping.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape(r#""quoted""#), "&quot;quoted&quot;");
        assert_eq!(html_escape("normal"), "normal");
    }
}
