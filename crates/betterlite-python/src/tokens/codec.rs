use std::sync::Arc;

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;
use secrecy::SecretString;

use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;
use betterlite_tokenizer::engine::selection::{CustomTokenizer, TokenizerType};

use super::convert::{DecodeRequest, EncodeRequest, LoadRequest, read};
use super::error::TokensError;
use super::runtime::TokensRuntime;
use super::store;

// LiteLLM's _select_tokenizer: a custom handle wins, then the model's tokenizer family, then the default cl100k encoding
async fn engine_for(
    runtime: &TokensRuntime,
    model: &str,
    handle: Option<u64>,
) -> Result<(Engine, TokenizerType), TokensError> {
    match handle {
        // a handle only ever holds a hugging face tokenizer, which the two builders load
        Some(handle) => Ok((store::engine(handle)?, TokenizerType::HuggingFace)),
        None => Ok(runtime.selector.select(model, None).await?),
    }
}

fn pretrained(identifier: &str, revision: &str, auth_token: Option<&str>) -> CustomTokenizer {
    CustomTokenizer {
        identifier: identifier.to_owned(),
        revision: revision.to_owned(),
        auth_token: auth_token.map(SecretString::from),
    }
}

async fn loaded(runtime: &TokensRuntime, wire: &LoadRequest) -> Result<Engine, TokensError> {
    match wire {
        LoadRequest::Pretrained {
            identifier,
            revision,
            auth_token,
        } => {
            let custom = pretrained(identifier, revision, auth_token.as_deref());
            let (engine, _kind) = runtime.selector.select("", Some(&custom)).await?;
            Ok(engine)
        }
        LoadRequest::Json { json } => Ok(Engine::HuggingFace(Arc::new(
            HuggingFaceEngine::from_bytes(json.as_bytes())?,
        ))),
    }
}

#[pyfunction]
pub(crate) fn encode_text(py: Python<'_>, request: &str) -> PyResult<Vec<u32>> {
    let wire: EncodeRequest = read(request)?;
    let encoded = py.detach(|| -> Result<Vec<u32>, TokensError> {
        let runtime = TokensRuntime::build()?;
        get_runtime().block_on(async {
            let (engine, _kind) = engine_for(&runtime, &wire.model, wire.custom_tokenizer).await?;
            engine.encode(&wire.text).map_err(TokensError::from)
        })
    })?;
    Ok(encoded)
}

#[pyfunction]
pub(crate) fn decode_tokens(py: Python<'_>, request: &str) -> PyResult<String> {
    let wire: DecodeRequest = read(request)?;
    let decoded = py.detach(|| -> Result<String, TokensError> {
        let runtime = TokensRuntime::build()?;
        get_runtime().block_on(async {
            let (engine, kind) = engine_for(&runtime, &wire.model, wire.custom_tokenizer).await?;
            if !wire.skip_special_tokens && matches!(kind, TokenizerType::HuggingFace) {
                return Err(TokensError::SkipSpecialTokens);
            }
            engine.decode(&wire.tokens).map_err(TokensError::from)
        })
    })?;
    Ok(decoded)
}

#[pyfunction]
pub(crate) fn load_tokenizer(py: Python<'_>, request: &str) -> PyResult<u64> {
    let wire: LoadRequest = read(request)?;
    let engine = py.detach(|| -> Result<Engine, TokensError> {
        let runtime = TokensRuntime::build()?;
        get_runtime().block_on(loaded(&runtime, &wire))
    })?;
    let tokenizer = store::insert(engine)?;
    Ok(tokenizer)
}

#[pyfunction]
pub(crate) fn free_tokenizer(tokenizer: u64) -> PyResult<()> {
    Ok(store::remove(tokenizer)?)
}

mod tests;
