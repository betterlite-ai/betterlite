use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt;

use crate::model_routes::ParamMap;

pub(super) const NOTE: &str = "Operator note (not from the user): the following was originally a mid-conversation system-role reminder.";

pub(super) const MODEL_COST: &str = r#"{
    "anthropic.claude-4-5-bedrock": {"supports_prompt_caching": true, "cache_creation_input_token_cost_above_1hr": 0.0000024},
    "anthropic.claude-3-5-sonnet": {"supports_prompt_caching": true},
    "anthropic.claude-no-cache": {"supports_prompt_caching": false},
    "openai.gpt-5": {"supports_prompt_caching": true}
}"#;

pub(super) const CLAUDE_3_5: &str = "anthropic.claude-3-5-sonnet";
pub(super) const CLAUDE_4_5: &str = "anthropic.claude-4-5-bedrock";

pub(super) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("the messages parse")
}

pub(super) fn params(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("the parameters parse")
}

pub(super) fn render(messages: &[Message]) -> Json {
    let rendered = serde_json::to_string(messages).expect("the messages render");
    serde_json::from_str(&rendered).expect("the rendered messages read back")
}

pub(super) fn render_params(parameters: &ParamMap) -> Json {
    let rendered = serde_json::to_string(parameters).expect("the parameters render");
    serde_json::from_str(&rendered).expect("the rendered parameters read back")
}

pub(super) fn blocks(system: &[Json]) -> Json {
    Json::List(system.to_vec())
}

pub(super) fn expected(source: &str) -> Json {
    serde_json::from_str(source).expect("the expected JSON parses")
}

// A JSON array assembled from one-line message literals.
pub(super) fn json_list(entries: &[&str]) -> String {
    format!("[{}]", entries.join(", "))
}

// The expected order, one message JSON per entry.
pub(super) fn timeline<S: AsRef<str>>(entries: &[S]) -> Json {
    let rendered: Vec<&str> = entries.iter().map(AsRef::as_ref).collect();
    expected(&json_list(&rendered))
}

pub(super) fn text_turn(text: &str) -> String {
    format!(
        r#"{{"role": "user", "content": [{{"type": "text", "text": "{NOTE}"}}, {{"type": "text", "text": "{text}"}}]}}"#
    )
}

pub(super) fn user_turn(text: &str) -> String {
    format!(r#"{{"role": "user", "content": "{text}"}}"#)
}

pub(super) fn assistant_turn(content: &str) -> String {
    format!(r#"{{"role": "assistant", "content": "{content}"}}"#)
}

#[derive(Clone, Default)]
pub(super) struct Warnings {
    written: Arc<Mutex<Vec<u8>>>,
}

impl Warnings {
    pub(super) fn install(&self) -> DefaultGuard {
        let writer = self.clone();
        let subscriber = fmt()
            .with_max_level(Level::WARN)
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    pub(super) fn captured(&self) -> String {
        let written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&written).into_owned()
    }
}

impl Write for Warnings {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
