use betterlite_anthropic::usage::{
    AnthropicUsage, ReasoningTokenCounter, calculate_usage, is_anthropic_usage_object,
};
use betterlite_models::decisions::Json;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use serde::{Deserialize, Serialize};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    usage: Json,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    completion_response: Option<Json>,
    #[serde(default)]
    speed: Option<String>,
}

struct LocalCounter {
    engine: TiktokenEngine,
}

impl ReasoningTokenCounter for LocalCounter {
    type Failure = EngineError;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure> {
        self.engine.encode(text).map(|ids| ids.len())
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let usage = calculate_usage(
            &input.usage,
            input.reasoning_content.as_deref(),
            input.completion_response.as_ref(),
            input.speed.as_deref(),
            &local_counter()?,
        )
        .map_err(|error| {
            Box::new(HelperError {
                class: "AnthropicUsage".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let answer = UsageAnswer {
            usage,
            is_anthropic_usage_object: is_anthropic_usage_object(&input.usage),
        };
        serde_json::to_value(answer).map_err(|error| {
            Box::new(HelperError {
                class: "AnthropicUsage".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn local_counter() -> Result<LocalCounter, Box<HelperError>> {
    let Some(data) = assets::bundled(Encoding::Cl100kBase) else {
        return Err(Box::new(HelperError {
            class: "AnthropicUsage".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "the cl100k vocabulary is unavailable".to_owned(),
        }));
    };
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).map_err(|error| {
        Box::new(HelperError {
            class: "AnthropicUsage".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(LocalCounter { engine })
}

#[derive(Serialize)]
struct UsageAnswer {
    usage: AnthropicUsage,
    is_anthropic_usage_object: bool,
}

mod tests;
