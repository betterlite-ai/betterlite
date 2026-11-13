mod fakes;
mod input;
mod selector;

use std::sync::Arc;

use betterlite_config::sections::semantic_embedding_inputs::DEFAULT_TIMEOUT_SECONDS;
use betterlite_router::semantic_embeddings::error::SemanticEmbeddingError;
use betterlite_router::semantic_embeddings::redis::redis_embedding;
use betterlite_router::semantic_embeddings::{
    EmbeddingMode, EmbeddingRequest, EmbeddingRoute, SemanticEmbeddingService,
    SemanticEmbeddingSettings,
};
use betterlite_tokenizer::engine::selection::Selector;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fakes::{CallLog, HelperCallFailure, HelperRouter, RecordingCalls};
use self::input::{Input, Method, ModelRow};
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        runtime.block_on(scenario(input))
    }
}

#[derive(Clone, Copy)]
enum Backend {
    Redis,
    Qdrant,
}

async fn scenario(input: &Input) -> HelperResult {
    let selector = Arc::new(selector::selector()?);
    let rows = input.model_list.as_deref();
    let redis = backend(input, Backend::Redis, rows, Arc::clone(&selector)).await;
    let qdrant = backend(input, Backend::Qdrant, rows, selector).await;
    Ok(json!({"redis": redis, "qdrant": qdrant}))
}

async fn backend<'input>(
    input: &'input Input,
    backend: Backend,
    rows: Option<&'input [ModelRow]>,
    selector: Arc<Selector>,
) -> Value {
    let calls = RecordingCalls {
        log: CallLog::default(),
        vectors: input.vectors.clone(),
        model: input.embedding_model.clone(),
        outcome: input.outcome,
    };
    let log = calls.log.clone();
    let router = rows
        .filter(|_| input.router_present)
        .map(|list| HelperRouter { rows: list });
    let service = SemanticEmbeddingService::new(
        settings(input),
        router,
        input.model_entries(),
        calls,
        selector,
    );
    let mode = match input.method {
        Method::Sync => EmbeddingMode::Sync,
        Method::Async => EmbeddingMode::Async,
    };
    let result = match backend {
        Backend::Redis => redis_embedding(&service, &input.prompt, input.metadata.as_ref(), mode)
            .await
            .map(|vector| json!(vector)),
        Backend::Qdrant => match service
            .prepare(&input.prompt, input.metadata.as_ref())
            .await
        {
            Ok(request) => service
                .call(&request, mode)
                .await
                .map(|response| response_json(&response.vectors, &response.model)),
            Err(error) => Err(error),
        },
    };
    outcome_json(&log, result)
}

fn settings(input: &Input) -> SemanticEmbeddingSettings {
    SemanticEmbeddingSettings {
        embedding_model: input.embedding_model.clone(),
        embedding_max_input_tokens: input.embedding_max_input_tokens,
        embedding_timeout: input.embedding_timeout,
        default_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
    }
}

fn outcome_json(
    log: &CallLog,
    result: Result<Value, SemanticEmbeddingError<HelperCallFailure>>,
) -> Value {
    let call = log.requests().first().map_or(Value::Null, call_json);
    match result {
        Ok(value) => json!({"call": call, "outcome": "ok", "value": value}),
        Err(error) => json!({"call": call, "outcome": "error", "status": error.status_code()}),
    }
}

fn call_json(request: &EmbeddingRequest) -> Value {
    json!({
        "routed": request.route == EmbeddingRoute::Router,
        "model": request.model,
        "input": request.input,
        "cache": {"no-store": request.cache.no_store, "no-cache": request.cache.no_cache},
        "metadata": request.metadata,
        "timeout": request.timeout,
        "num_retries": request.num_retries,
    })
}

fn response_json(vectors: &[Vec<f64>], model: &str) -> Value {
    let data: Vec<Value> = vectors
        .iter()
        .enumerate()
        .map(|(index, vector)| json!({"object": "embedding", "embedding": vector, "index": index}))
        .collect();
    json!({
        "object": "list",
        "data": data,
        "model": model,
        "usage": {"prompt_tokens": 0, "total_tokens": 0},
    })
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
