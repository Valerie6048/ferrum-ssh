//! AI command assistant: turns a natural-language request into a shell command.
//! Uses the Anthropic Messages API or any OpenAI-compatible chat endpoint.

use anyhow::{Context, Result, bail};

use crate::model::{AiProvider, Settings};

const SYSTEM: &str = "You convert requests into a single shell command for the user's terminal. \
Reply with only the command on one line (use && or pipes if needed), no explanation, no code fences. \
Prefer safe, non-destructive flags. If the request is unclear, reply with a harmless `echo` explaining what is missing.";

pub async fn suggest_command(s: &Settings, request: &str, context: &str) -> Result<String> {
    if s.ai_api_key.trim().is_empty() {
        bail!("add an API key in Settings → AI assistant");
    }
    let user = format!("Environment: {context}\nRequest: {request}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let text = match s.ai_provider {
        AiProvider::Anthropic => {
            let body = serde_json::json!({
                "model": s.ai_model,
                "max_tokens": 300,
                "system": SYSTEM,
                "messages": [{"role": "user", "content": user}],
            });
            let v: serde_json::Value = client
                .post(&s.ai_endpoint)
                .header("x-api-key", s.ai_api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await?
                .error_for_status()
                .context("AI request failed")?
                .json()
                .await?;
            v["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
        AiProvider::OpenAiCompatible => {
            let body = serde_json::json!({
                "model": s.ai_model,
                "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}],
                "max_tokens": 300,
            });
            let v: serde_json::Value = client
                .post(&s.ai_endpoint)
                .bearer_auth(s.ai_api_key.trim())
                .json(&body)
                .send()
                .await?
                .error_for_status()
                .context("AI request failed")?
                .json()
                .await?;
            v["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
    };
    Ok(clean(&text))
}

/// Strip code fences / prompts the model may add.
pub fn clean(text: &str) -> String {
    let t = text
        .trim()
        .trim_start_matches("```bash")
        .trim_start_matches("```sh")
        .trim_start_matches("```");
    let t = t.trim_end_matches("```").trim();
    let line = t
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    line.trim_start_matches("$ ").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleans_fenced_answers() {
        assert_eq!(super::clean("```bash\n$ ls -la\n```"), "ls -la");
        assert_eq!(super::clean("df -h"), "df -h");
    }
}
