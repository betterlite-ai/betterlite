use std::sync::Arc;

use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

type Failure = Box<crate::helpers::HelperError>;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    engine: EngineName,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    tokens: Option<Vec<u32>>,
    #[serde(default)]
    tokenizer_json: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Encode,
    Decode,
    Count,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
enum EngineName {
    #[serde(rename = "cl100k_base")]
    Cl100kBase,
    #[serde(rename = "o200k_base")]
    O200kBase,
    #[serde(rename = "p50k_base")]
    P50kBase,
    #[serde(rename = "p50k_edit")]
    P50kEdit,
    #[serde(rename = "o200k_harmony")]
    O200kHarmony,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "custom")]
    Custom,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let engine = build(input)?;
        match input.operation {
            Operation::Encode => serde_json::to_value(
                engine
                    .encode(&text(input)?)
                    .map_err(|error: EngineError| failure(error.to_string()))?,
            )
            .map_err(|error| failure(error.to_string())),
            Operation::Count => serde_json::to_value(
                engine
                    .encode(&text(input)?)
                    .map_err(|error: EngineError| failure(error.to_string()))?
                    .len(),
            )
            .map_err(|error| failure(error.to_string())),
            Operation::Decode => serde_json::to_value(
                engine
                    .decode(&tokens(input)?)
                    .map_err(|error: EngineError| failure(error.to_string()))?,
            )
            .map_err(|error| failure(error.to_string())),
        }
    }
}

fn build(input: &Input) -> Result<Engine, Failure> {
    match input.engine {
        EngineName::Cl100kBase => tiktoken(Encoding::Cl100kBase),
        EngineName::O200kBase => tiktoken(Encoding::O200kBase),
        EngineName::P50kBase => tiktoken(Encoding::P50kBase),
        EngineName::P50kEdit => tiktoken(Encoding::P50kEdit),
        EngineName::O200kHarmony => tiktoken(Encoding::O200kHarmony),
        EngineName::Anthropic => Ok(Engine::HuggingFace(Arc::new(
            HuggingFaceEngine::from_bytes(&assets::joined(assets::ANTHROPIC_PARTS))
                .map_err(|error: EngineError| failure(error.to_string()))?,
        ))),
        EngineName::Custom => {
            let json = input.tokenizer_json.as_deref().ok_or_else(|| {
                failure("tokenizer_json is required for the custom engine".to_owned())
            })?;
            Ok(Engine::HuggingFace(Arc::new(
                HuggingFaceEngine::from_bytes(json.as_bytes())
                    .map_err(|error: EngineError| failure(error.to_string()))?,
            )))
        }
    }
}

fn tiktoken(encoding: Encoding) -> Result<Engine, Failure> {
    let data = assets::bundled(encoding)
        .ok_or_else(|| failure(format!("{} is not a bundled encoding", encoding.name())))?;
    let engine = TiktokenEngine::from_bpe_file(encoding, &data)
        .map_err(|error: EngineError| failure(error.to_string()))?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}

fn text(input: &Input) -> Result<String, Failure> {
    input
        .text
        .clone()
        .ok_or_else(|| failure("text is required".to_owned()))
}

fn tokens(input: &Input) -> Result<Vec<u32>, Failure> {
    input
        .tokens
        .clone()
        .ok_or_else(|| failure("tokens are required".to_owned()))
}

fn failure(message: String) -> Failure {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
