//! AI analysis: sends a summary of CPU, memory and disk usage to one or more
//! large language models when the user asks for it, and keeps each reply.
//!
//! Supported providers: Claude, OpenAI, Gemini, Groq, Mistral, DeepSeek,
//! OpenRouter and a local Ollama server. Every provider except Claude speaks the
//! OpenAI-compatible chat-completions API. Each reads its own key from an
//! environment variable; keys are never stored, logged or displayed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::{json, Value};

use crate::stats::Snapshot;
use crate::widgets::fmt_bytes;

const MAX_TOKENS: u32 = 4096;

const SYSTEM_PROMPT: &str = "You are the diagnostics assistant built into DAEMON, a system \
monitor. You receive a snapshot of CPU, memory and disk usage from the user's own computer. \
Explain what stands out (sustained high CPU, memory or swap pressure, nearly full disks), say \
when everything looks healthy, and give a few short, practical suggestions. Keep it under 200 \
words. Use plain text with short lines and simple '-' bullets; no markdown headings or tables.";

/// Supported LLM providers, in the order shown in the panel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Provider {
    Anthropic,
    OpenAi,
    Gemini,
    Groq,
    Mistral,
    DeepSeek,
    OpenRouter,
    Ollama,
}

pub const PROVIDERS: [Provider; 8] = [
    Provider::Anthropic,
    Provider::OpenAi,
    Provider::Gemini,
    Provider::Groq,
    Provider::Mistral,
    Provider::DeepSeek,
    Provider::OpenRouter,
    Provider::Ollama,
];

impl Provider {
    /// Short name shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "CLAUDE",
            Provider::OpenAi => "OPENAI",
            Provider::Gemini => "GEMINI",
            Provider::Groq => "GROQ",
            Provider::Mistral => "MISTRAL",
            Provider::DeepSeek => "DEEPSEEK",
            Provider::OpenRouter => "OPENROUTER",
            Provider::Ollama => "OLLAMA",
        }
    }

    /// Environment variable holding this provider's API key (empty for Ollama, which needs none).
    pub fn key_var(self) -> &'static str {
        match self {
            Provider::Anthropic => "ANTHROPIC_API_KEY",
            Provider::OpenAi => "OPENAI_API_KEY",
            Provider::Gemini => "GEMINI_API_KEY",
            Provider::Groq => "GROQ_API_KEY",
            Provider::Mistral => "MISTRAL_API_KEY",
            Provider::DeepSeek => "DEEPSEEK_API_KEY",
            Provider::OpenRouter => "OPENROUTER_API_KEY",
            Provider::Ollama => "",
        }
    }

    /// Environment variable that overrides the default model.
    pub fn model_var(self) -> &'static str {
        match self {
            Provider::Anthropic => "ANTHROPIC_MODEL",
            Provider::OpenAi => "OPENAI_MODEL",
            Provider::Gemini => "GEMINI_MODEL",
            Provider::Groq => "GROQ_MODEL",
            Provider::Mistral => "MISTRAL_MODEL",
            Provider::DeepSeek => "DEEPSEEK_MODEL",
            Provider::OpenRouter => "OPENROUTER_MODEL",
            Provider::Ollama => "OLLAMA_MODEL",
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => "claude-opus-5-5",
            Provider::OpenAi => "gpt-4o-mini",
            Provider::Gemini => "gemini-2.5-flash",
            Provider::Groq => "llama-3.3-70b-versatile",
            Provider::Mistral => "mistral-small-latest",
            Provider::DeepSeek => "deepseek-chat",
            Provider::OpenRouter => "openrouter/auto",
            Provider::Ollama => "llama3.2",
        }
    }

    /// Where to get a key (or, for Ollama, the server), shown when it is missing.
    pub fn signup(self) -> &'static str {
        match self {
            Provider::Anthropic => "console.anthropic.com",
            Provider::OpenAi => "platform.openai.com/api-keys",
            Provider::Gemini => "aistudio.google.com/apikey",
            Provider::Groq => "console.groq.com/keys",
            Provider::Mistral => "console.mistral.ai/api-keys",
            Provider::DeepSeek => "platform.deepseek.com/api_keys",
            Provider::OpenRouter => "openrouter.ai/keys",
            Provider::Ollama => "ollama.com/download",
        }
    }

    /// Whether the provider needs an API key (a local Ollama server does not).
    pub fn needs_key(self) -> bool {
        self != Provider::Ollama
    }

    /// Whether the user has set this provider up: its key is set, or for Ollama,
    /// `OLLAMA_MODEL` or `OLLAMA_HOST` is. Only these run with "analyze all".
    pub fn configured(self) -> bool {
        if self.needs_key() {
            return self.key().is_some();
        }
        ["OLLAMA_MODEL", "OLLAMA_HOST"].iter().any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
    }

    /// Chat endpoint. Ollama honors `OLLAMA_HOST` like the Ollama CLI does.
    pub fn url(self) -> String {
        match self {
            Provider::Anthropic => "https://api.anthropic.com/v1/messages".into(),
            Provider::OpenAi => "https://api.openai.com/v1/chat/completions".into(),
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions".into(),
            Provider::Groq => "https://api.groq.com/openai/v1/chat/completions".into(),
            Provider::Mistral => "https://api.mistral.ai/v1/chat/completions".into(),
            Provider::DeepSeek => "https://api.deepseek.com/chat/completions".into(),
            Provider::OpenRouter => "https://openrouter.ai/api/v1/chat/completions".into(),
            Provider::Ollama => {
                format!("{}/v1/chat/completions", ollama_base(std::env::var("OLLAMA_HOST").ok().as_deref()))
            }
        }
    }

    /// The model to use: the `*_MODEL` env override if set, else the default.
    pub fn model(self) -> String {
        std::env::var(self.model_var())
            .ok()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.default_model().to_string())
    }

    /// This provider's API key from the environment, if set and non-empty.
    /// A provider that needs no key always yields an empty one.
    pub fn key(self) -> Option<String> {
        if !self.needs_key() {
            return Some(String::new());
        }
        std::env::var(self.key_var()).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
    }
}

/// Base URL of the Ollama server from an `OLLAMA_HOST`-style value
/// (`host`, `host:port` or a full URL); defaults to the local server.
fn ollama_base(host: Option<&str>) -> String {
    let host = host.map(str::trim).filter(|h| !h.is_empty()).unwrap_or("127.0.0.1:11434");
    // A full URL is used as given; a bare host gets Ollama's default port.
    let base = if host.contains("://") {
        host.to_string()
    } else if host.contains(':') {
        format!("http://{host}")
    } else {
        format!("http://{host}:11434")
    };
    // `0.0.0.0` is a listen address; connect to loopback instead.
    base.replacen("://0.0.0.0", "://127.0.0.1", 1).trim_end_matches('/').to_string()
}

#[derive(Clone, Default)]
pub enum Status {
    #[default]
    Idle,
    Running,
    Done(String),
    Error(String),
}

/// The latest run for one provider.
#[derive(Clone, Default)]
pub struct Run {
    pub status: Status,
    pub model: String,
    /// The summary sent with the request.
    pub sent: String,
    pub finished_at: Option<chrono::DateTime<chrono::Local>>,
    /// How long the provider took to answer.
    pub took: Option<Duration>,
}

#[derive(Clone, Default)]
pub struct AiState {
    pub runs: HashMap<Provider, Run>,
}

impl AiState {
    pub fn run(&self, p: Provider) -> Run {
        self.runs.get(&p).cloned().unwrap_or_default()
    }

    pub fn running(&self, p: Provider) -> bool {
        self.runs.get(&p).is_some_and(|r| matches!(r.status, Status::Running))
    }
}

pub type SharedAi = Arc<Mutex<AiState>>;

fn pct(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32 * 100.0
    }
}

/// Plain-text summary of CPU, memory and disk usage. Contains no host name,
/// user name or network addresses.
pub fn summary(s: &Snapshot) -> String {
    let mut out = String::new();
    let avg = if s.cpu_history.is_empty() { s.cpu_total } else { s.cpu_history.iter().sum::<f32>() / s.cpu_history.len() as f32 };
    let peak = s.cpu_history.iter().cloned().fold(s.cpu_total, f32::max);
    out += &format!("OS: {}\n", s.os);
    out += &format!("CPU: {} ({} logical cores)\n", s.cpu_brand, s.cores.len());
    out += &format!("CPU usage now: {:.0}%, recent average: {avg:.0}%, recent peak: {peak:.0}%\n", s.cpu_total);
    let cores: Vec<String> = s.cores.iter().map(|c| format!("{c:.0}")).collect();
    out += &format!("Per-core usage (%): {}\n", cores.join(", "));
    out += &format!(
        "Memory: {} used of {} ({:.0}%)\n",
        fmt_bytes(s.mem_used),
        fmt_bytes(s.mem_total),
        pct(s.mem_used, s.mem_total)
    );
    out += &format!(
        "Swap: {} used of {} ({:.0}%)\n",
        fmt_bytes(s.swap_used),
        fmt_bytes(s.swap_total),
        pct(s.swap_used, s.swap_total)
    );
    out += "Disks:\n";
    for d in &s.disks {
        out += &format!(
            "- {}: {} used of {} ({:.0}%)\n",
            d.mount,
            fmt_bytes(d.used),
            fmt_bytes(d.total),
            pct(d.used, d.total)
        );
    }
    out += &format!("Uptime: {} hours\n", s.uptime / 3600);
    out
}

fn user_prompt(summary: &str) -> String {
    format!("Here is my system snapshot. What do you notice?\n\n{summary}")
}

fn request_body(provider: Provider, model: &str, summary: &str) -> Value {
    match provider {
        Provider::Anthropic => json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "system": SYSTEM_PROMPT,
            "messages": [{ "role": "user", "content": user_prompt(summary) }]
        }),
        // OpenAI-compatible chat completions.
        _ => json!({
            "model": model,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": user_prompt(summary) }
            ]
        }),
    }
}

/// Pulls the reply text out of a provider's response.
fn extract_text(provider: Provider, resp: &Value) -> Result<String, String> {
    match provider {
        Provider::Anthropic => {
            if resp["stop_reason"] == "refusal" {
                return Err("The model declined to answer this request.".into());
            }
            let text: Vec<&str> = resp["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect();
            if text.is_empty() {
                return Err("The response contained no text.".into());
            }
            let mut out = text.join("\n").trim().to_string();
            if resp["stop_reason"] == "max_tokens" {
                out += "\n\n[reply cut off at the token limit]";
            }
            Ok(out)
        }
        _ => {
            let choice = &resp["choices"][0];
            if let Some(refusal) = choice["message"]["refusal"].as_str() {
                if !refusal.is_empty() {
                    return Err(format!("The model declined: {refusal}"));
                }
            }
            let text = choice["message"]["content"].as_str().unwrap_or("").trim().to_string();
            if text.is_empty() {
                return Err("The response contained no text.".into());
            }
            let mut out = text;
            if choice["finish_reason"] == "length" {
                out += "\n\n[reply cut off at the token limit]";
            }
            Ok(out)
        }
    }
}

/// Readable message for an HTTP error status, using the API's error body when present.
/// Most providers return `{ "error": { "message": ... } }`; Gemini sometimes wraps it in an array.
fn http_error(provider: Provider, code: u16, body: &Value) -> String {
    let err = if body.is_array() { &body[0]["error"] } else { &body["error"] };
    let detail = err["message"].as_str().or_else(|| err.as_str()).unwrap_or("");
    let hint = match code {
        401 => "API key rejected.",
        403 => "This API key is not allowed to use this model.",
        404 => "Model not found. Check the model name.",
        429 => "Rate limit or quota reached. Check your plan and try again.",
        529 => "The API is overloaded. Try again shortly.",
        c if c >= 500 => "The API had a server error. Try again shortly.",
        _ => "The request failed.",
    };
    let where_ = if provider.needs_key() { provider.key_var() } else { provider.model_var() };
    if detail.is_empty() {
        format!("HTTP {code}: {hint} ({where_})")
    } else {
        format!("HTTP {code}: {hint}\n{detail}")
    }
}

fn call_api(provider: Provider, key: &str, model: &str, summary: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build();
    let mut req = agent.post(&provider.url()).set("content-type", "application/json");
    req = match provider {
        Provider::Anthropic => req.set("x-api-key", key).set("anthropic-version", "2023-06-01"),
        _ if key.is_empty() => req,
        _ => req.set("authorization", &format!("Bearer {key}")),
    };
    if provider == Provider::OpenRouter {
        // Optional attribution headers OpenRouter uses to identify the calling app.
        req = req.set("x-title", "DAEMON").set("http-referer", "https://github.com/fcopensource/daemon");
    }
    match req.send_json(request_body(provider, model, summary)) {
        Ok(resp) => {
            let body: Value = resp.into_json().map_err(|e| format!("Could not read the response: {e}"))?;
            extract_text(provider, &body)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body: Value = resp.into_json().unwrap_or(Value::Null);
            Err(http_error(provider, code, &body))
        }
        Err(e) if provider == Provider::Ollama => {
            Err(format!("Could not reach Ollama at {}: {e}\nIs `ollama serve` running?", provider.url()))
        }
        Err(e) => Err(format!("Could not reach {}: {e}", provider.label())),
    }
}

/// Starts an analysis with `provider` on a background thread unless one is
/// already running for it. Returns whether a request was started.
pub fn analyze(shared: &SharedAi, provider: Provider, snapshot: &Snapshot, ctx: egui::Context) -> bool {
    let Some(key) = provider.key() else { return false };
    let model = provider.model();
    let summary = summary(snapshot);
    {
        let mut st = shared.lock().unwrap();
        if st.running(provider) {
            return false;
        }
        st.runs.insert(provider, Run { status: Status::Running, model: model.clone(), sent: summary.clone(), ..Run::default() });
    }
    let shared = shared.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        let status = match call_api(provider, &key, &model, &summary) {
            Ok(text) => Status::Done(text),
            Err(e) => Status::Error(e),
        };
        let mut st = shared.lock().unwrap();
        let run = st.runs.entry(provider).or_default();
        run.status = status;
        run.finished_at = Some(chrono::Local::now());
        run.took = Some(started.elapsed());
        drop(st);
        ctx.request_repaint();
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::DiskInfo;

    fn snapshot() -> Snapshot {
        let mut s = Snapshot::default();
        s.hostname = "secret-host".into();
        s.cpu_brand = "Test CPU".into();
        s.cpu_total = 42.0;
        s.cores = vec![40.0, 44.0];
        s.mem_used = 4 << 30;
        s.mem_total = 16 << 30;
        s.disks = vec![DiskInfo { name: "SSD".into(), mount: "C:\\".into(), used: 90 << 30, total: 100 << 30 }];
        s
    }

    #[test]
    fn summary_has_stats_but_no_hostname() {
        let text = summary(&snapshot());
        assert!(text.contains("Test CPU (2 logical cores)"));
        assert!(text.contains("CPU usage now: 42%"));
        assert!(text.contains("(25%)"), "{text}");
        assert!(text.contains("C:\\") && text.contains("(90%)"), "{text}");
        assert!(!text.contains("secret-host"));
    }

    #[test]
    fn body_shape_differs_by_provider() {
        let a = request_body(Provider::Anthropic, "claude-x", "CPU: 1%");
        assert_eq!(a["model"], "claude-x");
        assert_eq!(a["system"], SYSTEM_PROMPT);
        assert!(a["messages"][0]["content"].as_str().unwrap().contains("CPU: 1%"));

        // Every other provider uses the OpenAI shape: system prompt as the first message.
        for p in PROVIDERS.into_iter().filter(|&p| p != Provider::Anthropic) {
            let o = request_body(p, "m-x", "CPU: 1%");
            assert_eq!(o["model"], "m-x");
            assert!(o["system"].is_null());
            assert_eq!(o["messages"][0]["role"], "system");
            assert_eq!(o["messages"][1]["role"], "user");
        }
    }

    #[test]
    fn extracts_text_for_both_api_shapes() {
        let a = json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "ok a"}]});
        assert_eq!(extract_text(Provider::Anthropic, &a).unwrap(), "ok a");

        let o = json!({"choices": [{"finish_reason": "stop", "message": {"content": "ok o"}}]});
        assert_eq!(extract_text(Provider::OpenAi, &o).unwrap(), "ok o");
        assert_eq!(extract_text(Provider::Gemini, &o).unwrap(), "ok o");
        assert_eq!(extract_text(Provider::Ollama, &o).unwrap(), "ok o");
    }

    #[test]
    fn handles_refusal_truncation_and_errors() {
        assert!(extract_text(Provider::Anthropic, &json!({"stop_reason": "refusal", "content": []})).is_err());
        let cut = json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "abc"}]});
        assert!(extract_text(Provider::Anthropic, &cut).unwrap().ends_with("[reply cut off at the token limit]"));

        let o_cut = json!({"choices": [{"finish_reason": "length", "message": {"content": "abc"}}]});
        assert!(extract_text(Provider::OpenAi, &o_cut).unwrap().ends_with("[reply cut off at the token limit]"));
        let o_ref = json!({"choices": [{"finish_reason": "stop", "message": {"content": "", "refusal": "no"}}]});
        assert!(extract_text(Provider::OpenAi, &o_ref).is_err());

        let e = http_error(Provider::OpenAi, 401, &json!({"error": {"message": "bad key"}}));
        assert!(e.contains("401") && e.contains("bad key"));
        let g = http_error(Provider::Gemini, 400, &json!([{"error": {"message": "bad model"}}]));
        assert!(g.contains("bad model"), "{g}");
        let n = http_error(Provider::Groq, 429, &Value::Null);
        assert!(n.contains("GROQ_API_KEY"), "{n}");
    }

    #[test]
    fn providers_have_distinct_settings() {
        use std::collections::HashSet;
        let keyed: Vec<_> = PROVIDERS.into_iter().filter(|p| p.needs_key()).collect();
        assert_eq!(keyed.iter().map(|p| p.key_var()).collect::<HashSet<_>>().len(), keyed.len());
        assert_eq!(PROVIDERS.iter().map(|p| p.model_var()).collect::<HashSet<_>>().len(), PROVIDERS.len());
        assert_eq!(PROVIDERS.iter().map(|p| p.label()).collect::<HashSet<_>>().len(), PROVIDERS.len());
        assert!(PROVIDERS.iter().all(|p| p.url().starts_with("http")));
        assert_eq!(Provider::Ollama.key(), Some(String::new()));
    }

    #[test]
    fn ollama_host_parsing() {
        assert_eq!(ollama_base(None), "http://127.0.0.1:11434");
        assert_eq!(ollama_base(Some("")), "http://127.0.0.1:11434");
        assert_eq!(ollama_base(Some("0.0.0.0")), "http://127.0.0.1:11434");
        assert_eq!(ollama_base(Some("gpu-box:8080")), "http://gpu-box:8080");
        assert_eq!(ollama_base(Some("https://ollama.example.com/")), "https://ollama.example.com");
        assert_eq!(ollama_base(Some("http://10.0.0.5:11434")), "http://10.0.0.5:11434");
    }
}
