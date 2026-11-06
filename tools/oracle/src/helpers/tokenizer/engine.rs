use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use std::sync::Arc;
type Failure = Box<crate::helpers::HelperError>;
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct Input {
    operation: Operation,
    engine: EngineName,
    text: Option<String>,
    tokens: Option<Vec<u32>>,
    tokenizer_json: Option<String>,
}
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Encode,
    Decode,
}
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum EngineName {
    Cl100kBase,
    O200kBase,
    P50kBase,
    P50kEdit,
    O200kHarmony,
    Anthropic,
    Custom,
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let engine = build(input)?;
        match input.operation {
            Operation::Encode => json(
                engine
                    .encode(&text(input)?)
                    .map_err(|error: EngineError| failure(error.to_string()))?,
            ),
            Operation::Decode => json(
                engine
                    .decode(&tokens(input)?)
                    .map_err(|error: EngineError| failure(error.to_string()))?,
            ),
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
        EngineName::Anthropic => huggingface(&assets::joined(assets::ANTHROPIC_PARTS)),
        EngineName::Custom => input.tokenizer_json.as_deref().map_or_else(
            || Err(failure("tokenizer_json is required".to_owned())),
            |document| huggingface(document.as_bytes()),
        ),
    }
}
fn tiktoken(encoding: Encoding) -> Result<Engine, Failure> {
    let data = assets::bundled(encoding)
        .ok_or_else(|| failure(format!("{} is not bundled", encoding.name())))?;
    let engine = TiktokenEngine::from_bpe_file(encoding, &data)
        .map_err(|error: EngineError| failure(error.to_string()))?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}
fn huggingface(bytes: &[u8]) -> Result<Engine, Failure> {
    let engine = HuggingFaceEngine::from_bytes(bytes)
        .map_err(|error: EngineError| failure(error.to_string()))?;
    Ok(Engine::HuggingFace(Arc::new(engine)))
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
fn json<T>(payload: T) -> crate::helpers::HelperResult
where
    T: serde::Serialize,
{
    match serde_json::to_value(payload) {
        Ok(value) => Ok(value),
        Err(error) => Err(failure(error.to_string())),
    }
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
