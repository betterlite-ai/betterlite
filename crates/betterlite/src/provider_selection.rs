use std::collections::BTreeSet;
use std::io::{BufRead, Write};

use crate::terminal_styling::supports_color;

pub mod fallback;
pub mod interactive;
pub mod raw_mode;

pub struct Provider {
    pub id: ProviderId,
    pub name: &'static str,
    pub description: &'static str,
    pub env_key: Option<&'static str>,
    pub key_hint: Option<&'static str>,
    pub test_model: Option<&'static str>,
    pub models: &'static [&'static str],
    pub needs_api_base: bool,
    pub api_base_hint: Option<&'static str>,
    pub api_version: Option<&'static str>,
    pub extra_keys: &'static [&'static str],
    pub extra_hints: &'static [&'static str],
    pub api_base: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderId {
    OpenAi,
    Anthropic,
    Gemini,
    Azure,
    Bedrock,
    Ollama,
}

impl ProviderId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Azure => "azure",
            Self::Bedrock => "bedrock",
            Self::Ollama => "ollama",
        }
    }
}

pub const PROVIDERS: [Provider; 6] = [
    Provider {
        id: ProviderId::OpenAi,
        name: "OpenAI",
        description: "GPT-4o, GPT-4o-mini, o3-mini",
        env_key: Some("OPENAI_API_KEY"),
        key_hint: Some("sk-..."),
        test_model: Some("gpt-4o-mini"),
        models: &["gpt-4o", "gpt-4o-mini"],
        needs_api_base: false,
        api_base_hint: None,
        api_version: None,
        extra_keys: &[],
        extra_hints: &[],
        api_base: None,
    },
    Provider {
        id: ProviderId::Anthropic,
        name: "Anthropic",
        description: "Claude Fable 5.1, Fable 5, Opus 5.5, Opus 5, Opus 4.8, Opus 4.7, Opus 4.6, Sonnet 5.5, Sonnet 5, Sonnet 4.6, Haiku 4.5",
        env_key: Some("ANTHROPIC_API_KEY"),
        key_hint: Some("sk-ant-..."),
        test_model: Some("claude-haiku-4-5-20251001"),
        models: &[
            "claude-fable-5-1",
            "claude-fable-5",
            "claude-opus-5-5",
            "claude-opus-5",
            "claude-sonnet-5-5",
            "claude-sonnet-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-haiku-4-5-20251001",
        ],
        needs_api_base: false,
        api_base_hint: None,
        api_version: None,
        extra_keys: &[],
        extra_hints: &[],
        api_base: None,
    },
    Provider {
        id: ProviderId::Gemini,
        name: "Google Gemini",
        description: "Gemini 2.0 Flash, Gemini 2.5 Pro",
        env_key: Some("GEMINI_API_KEY"),
        key_hint: Some("AIza..."),
        test_model: Some("gemini/gemini-2.0-flash"),
        models: &["gemini/gemini-2.0-flash", "gemini/gemini-2.5-pro"],
        needs_api_base: false,
        api_base_hint: None,
        api_version: None,
        extra_keys: &[],
        extra_hints: &[],
        api_base: None,
    },
    Provider {
        id: ProviderId::Azure,
        name: "Azure OpenAI",
        description: "GPT-4o via Azure",
        env_key: Some("AZURE_AI_API_KEY"),
        key_hint: Some("your-azure-key"),
        test_model: None,
        models: &[],
        needs_api_base: true,
        api_base_hint: Some("https://<resource>.openai.azure.com/"),
        api_version: Some("2024-07-01-preview"),
        extra_keys: &[],
        extra_hints: &[],
        api_base: None,
    },
    Provider {
        id: ProviderId::Bedrock,
        name: "AWS Bedrock",
        description: "Claude 3.5, Llama 3 via AWS",
        env_key: Some("AWS_ACCESS_KEY_ID"),
        key_hint: Some("AKIA..."),
        test_model: None,
        models: &["bedrock/anthropic.claude-haiku-4-5-20251001-v1:0"],
        needs_api_base: false,
        api_base_hint: None,
        api_version: None,
        extra_keys: &["AWS_SECRET_ACCESS_KEY", "AWS_REGION_NAME"],
        extra_hints: &["your-secret-key", "us-east-1"],
        api_base: None,
    },
    Provider {
        id: ProviderId::Ollama,
        name: "Ollama",
        description: "Local models (llama3.2, mistral, etc.)",
        env_key: None,
        key_hint: None,
        test_model: None,
        models: &["ollama/llama3.2", "ollama/mistral"],
        needs_api_base: false,
        api_base_hint: None,
        api_version: None,
        extra_keys: &[],
        extra_hints: &[],
        api_base: Some("http://localhost:11434"),
    },
];

#[derive(Debug, thiserror::Error)]
pub enum SelectError {
    #[error("provider selection was cancelled")]
    Cancelled,
    #[error(transparent)]
    Fallback(#[from] fallback::FallbackError),
}

pub fn select_providers(
    no_color: bool,
    fallback_reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<Vec<&'static Provider>, SelectError> {
    let supports = supports_color(no_color);
    dispatch_selection(
        raw_mode::TtyRawMode::open,
        fallback_reader,
        writer,
        supports,
    )
}

fn dispatch_selection<S, F>(
    open_raw: F,
    fallback_reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    supports: bool,
) -> Result<Vec<&'static Provider>, SelectError>
where
    S: interactive::ByteSource,
    F: FnOnce() -> Result<S, raw_mode::RawModeError>,
{
    match open_raw() {
        Ok(bytes) => {
            let mut key_source = interactive::TtyKeySource { bytes };
            match interactive::select_interactive(&mut key_source, writer, supports) {
                Ok(indices) => Ok(providers_at(&indices)),
                Err(interactive::InteractiveError::Cancelled) => Err(SelectError::Cancelled),
                Err(interactive::InteractiveError::Io(_)) => {
                    drop(key_source);
                    Ok(providers_at(&fallback::select_fallback(
                        fallback_reader,
                        writer,
                        supports,
                    )?))
                }
            }
        }
        Err(_) => Ok(providers_at(&fallback::select_fallback(
            fallback_reader,
            writer,
            supports,
        )?)),
    }
}

fn providers_at(indices: &BTreeSet<usize>) -> Vec<&'static Provider> {
    indices
        .iter()
        .filter_map(|&index| PROVIDERS.get(index))
        .collect()
}

mod tests;
