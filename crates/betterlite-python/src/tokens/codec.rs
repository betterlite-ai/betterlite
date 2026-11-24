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
    // a handle only ever holds a hugging face tokenizer, which the two builders load
    if let Some(id) = handle {
        return Ok((store::engine(id)?, TokenizerType::HuggingFace));
    }
    Ok(runtime.selector.select(model, None).await?)
}

fn pretrained(
    identifier: &str,
    revision: &str,
    auth_token: Option<SecretString>,
) -> CustomTokenizer {
    CustomTokenizer {
        identifier: identifier.to_owned(),
        revision: revision.to_owned(),
        auth_token,
    }
}

async fn loaded(runtime: &TokensRuntime, wire: &LoadRequest) -> Result<Engine, TokensError> {
    match wire {
        LoadRequest::Pretrained {
            identifier,
            revision,
            auth_token,
        } => {
            let custom = pretrained(identifier, revision, auth_token.clone());
            let (engine, _kind) = runtime.selector.select("", Some(&custom)).await?;
            Ok(engine)
        }
        LoadRequest::Json { json } => Ok(Engine::HuggingFace(Arc::new(
            HuggingFaceEngine::from_bytes(json.as_bytes())?,
        ))),
    }
}

// a request runs off the interpreter thread: the engine call inside differs, the runtime build does not
fn on_runtime<T, F>(py: Python<'_>, run: F) -> Result<T, TokensError>
where
    T: Send,
    F: FnOnce(&TokensRuntime) -> Result<T, TokensError> + Send,
{
    py.detach(|| {
        let runtime = TokensRuntime::build()?;
        run(&runtime)
    })
}

// one engine call off the interpreter thread: the request types differ, the runtime build and selection do not
fn with_engine<R, F>(
    py: Python<'_>,
    model: &str,
    custom_tokenizer: Option<u64>,
    run: F,
) -> Result<R, TokensError>
where
    R: Send,
    F: FnOnce(&Engine, TokenizerType) -> Result<R, TokensError> + Send,
{
    on_runtime(py, |runtime| {
        get_runtime().block_on(async {
            let (engine, kind) = engine_for(runtime, model, custom_tokenizer).await?;
            run(&engine, kind)
        })
    })
}

#[pyfunction]
pub(crate) fn encode_text(py: Python<'_>, request: &str) -> PyResult<Vec<u32>> {
    let wire: EncodeRequest = read(request)?;
    let encoded = with_engine(py, &wire.model, wire.custom_tokenizer, |engine, _kind| {
        engine.encode(&wire.text).map_err(TokensError::from)
    })?;
    Ok(encoded)
}

#[pyfunction]
pub(crate) fn decode_tokens(py: Python<'_>, request: &str) -> PyResult<String> {
    let wire: DecodeRequest = read(request)?;
    let decoded = with_engine(py, &wire.model, wire.custom_tokenizer, |engine, kind| {
        if !wire.skip_special_tokens && matches!(kind, TokenizerType::HuggingFace) {
            return Err(TokensError::SkipSpecialTokens);
        }
        engine.decode(&wire.tokens).map_err(TokensError::from)
    })?;
    Ok(decoded)
}

#[pyfunction]
pub(crate) fn load_tokenizer(py: Python<'_>, request: &str) -> PyResult<u64> {
    let wire: LoadRequest = read(request)?;
    let engine = on_runtime(py, |runtime| get_runtime().block_on(loaded(runtime, &wire)))?;
    Ok(store::insert(engine)?)
}

#[pyfunction]
pub(crate) fn free_tokenizer(tokenizer: u64) -> PyResult<()> {
    store::remove(tokenizer)?;
    Ok(())
}

mod tests;
