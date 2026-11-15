use std::collections::BTreeMap;

use serde::Serialize;

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::dialect::ModelFacts;
use crate::cache_defaults::CacheFacts;
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::Message;

pub(super) const MODEL: &str = "anthropic/claude-sonnet-4-5";
pub(super) const BARE_MODEL: &str = "claude-sonnet-4-5";
pub(super) const OPENAI_MODEL: &str = "openai/gpt-5.6";

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> Object {
    json(text).as_object().cloned().unwrap()
}

pub(super) fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn system(text: &str) -> Option<SystemPrompt> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn wire<T>(value: &T) -> Json
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).unwrap();
    serde_json::from_value(encoded).unwrap()
}

#[derive(Clone, Default)]
pub(super) struct Facts {
    supports: bool,
    providers: BTreeMap<String, String>,
    flags: BTreeMap<String, bool>,
}

impl Facts {
    pub(super) fn claude() -> Self {
        Self::routed(MODEL, "anthropic")
    }

    pub(super) fn routed(model: &str, provider: &str) -> Self {
        Self {
            supports: true,
            providers: BTreeMap::from([(model.to_owned(), provider.to_owned())]),
            flags: BTreeMap::new(),
        }
    }

    pub(super) fn without_support() -> Self {
        Self {
            supports: false,
            providers: BTreeMap::from([(MODEL.to_owned(), "anthropic".to_owned())]),
            flags: BTreeMap::new(),
        }
    }

    pub(super) fn none() -> Self {
        Self::default()
    }

    pub(super) fn with_flag(mut self, model: &str, flag: bool) -> Self {
        self.flags.insert(model.to_owned(), flag);
        self
    }
}

impl ModelFacts for Facts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        self.flags.get(model).copied()
    }

    fn provider(&self, model: &str) -> Option<String> {
        self.providers.get(model).cloned()
    }
}

impl CacheFacts for Facts {
    fn supports_prompt_caching(&self, _model: &str, _provider: &str) -> bool {
        self.supports
    }
}

// The two default points LiteLLM synthesizes for an ephemeral cache.
pub(super) const DEFAULT_POINTS: &str = r#"[
    {"location": "message", "role": "system", "index": null, "control": {"type": "ephemeral"}},
    {"location": "message", "role": null, "index": -1, "control": {"type": "ephemeral"}}
]"#;

pub(super) const MARKED_CHAT_USER: &str = r#"[{
    "role": "user",
    "content": "hi",
    "cache_control": {"type": "ephemeral"}
}]"#;

pub(super) const MARKED_SYSTEM: &str = r#"[{
    "type": "text",
    "text": "be nice",
    "cache_control": {"type": "ephemeral"}
}]"#;
