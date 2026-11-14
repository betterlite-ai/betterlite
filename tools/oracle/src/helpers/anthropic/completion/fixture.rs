use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_anthropic::completion::prompt::{CustomPromptEntry, PromptSource};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use chrono::{DateTime, TimeZone as _, Utc};
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{HelperError, OrHelperError as _};

// the fixed clock stands in for time.time(), and the Python side reports only whether created is a recent Unix time
pub(super) struct Fixture {
    pub(super) runtime: Runtime,
    factory: PromptFactory,
    now: DateTime<Utc>,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, Box<HelperError>> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let now = Utc
            .with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
            .single()
            .ok_or("the fixed clock is not a valid time")
            .or_value_error()?;
        let factory = PromptFactory {
            cache: Mutex::new(builtin_entries()),
            hub: Hub::Unreachable,
        };
        Ok(Self {
            runtime,
            factory,
            now,
        })
    }

    pub(super) fn source<'fixture>(
        &'fixture self,
        custom_prompt_dict: &'fixture BTreeMap<String, CustomPromptEntry>,
    ) -> PromptSource<'fixture> {
        PromptSource {
            custom_prompt_dict,
            prompts: &self.factory,
            now: self.now,
        }
    }
}

// litellm.encoding is tiktoken's cl100k_base, which the oracle gives the explicit engine too
pub(super) fn tiktoken() -> Result<Engine, Box<HelperError>> {
    let data = assets::bundled(Encoding::Cl100kBase)
        .ok_or("cl100k_base is not bundled")
        .or_value_error()?;
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).or_value_error()?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}

pub(super) fn blank_response(content: &str) -> Result<ChatResponse, Box<HelperError>> {
    serde_json::from_value(json!({
        "id": "chatcmpl-kept",
        "object": "chat.completion",
        "created": 1,
        "model": "unset",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": null,
        }],
    }))
    .or_value_error()
}
