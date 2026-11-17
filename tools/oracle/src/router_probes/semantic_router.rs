use std::sync::Arc;

use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::semantic_router::service::{AutoRouter, AutoRouterSettings};
use betterlite_server::routing::semantic_router::{CallerRequest, decide};
use futures_util::future::join_all;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

use self::recorder::{Recorder, call_counts, delta};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod recorder;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub config: String,
    pub default_model: String,
    pub embedding_model: String,
    #[serde(default)]
    pub max_input_chars: Option<i64>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub metadata: Option<Json>,
    #[serde(default)]
    pub litellm_metadata: Option<Json>,
    #[serde(default)]
    pub litellm_params_metadata: Option<Json>,
    #[serde(default)]
    pub litellm_session_id: Option<Json>,
    #[serde(default)]
    pub litellm_trace_id: Option<Json>,
    #[serde(default)]
    pub turn_off_message_logging: Option<Json>,
    #[serde(default)]
    pub vectors: IndexMap<String, Vec<Json>>,
    #[serde(default)]
    pub document_failures: usize,
    #[serde(default)]
    pub query_failures: usize,
    #[serde(default = "default_one")]
    pub rounds: usize,
    #[serde(default = "default_one")]
    pub concurrent: usize,
    #[serde(default)]
    pub pad_bytes: usize,
}

const fn default_one() -> usize {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub accepted: bool,
    pub code: Option<String>,
    pub rounds: Vec<Round>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub failed: bool,
    pub model: Option<String>,
    pub documents: Vec<DocumentCall>,
    pub queries: Vec<QueryCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentCall {
    pub model: String,
    pub input: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryCall {
    pub model: String,
    pub input: Vec<String>,
    pub metadata: IndexMap<String, Json>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub proxy_server_request: Json,
    pub turn_off_message_logging: Option<bool>,
    pub litellm_session_id: Option<String>,
    pub litellm_trace_id: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let recorder = Arc::new(Recorder::new(
        vectors(&input.vectors)?,
        input.document_failures,
        input.query_failures,
    ));
    let settings = AutoRouterSettings {
        config_path: None,
        config_json: Some(format!("{}{}", input.config, " ".repeat(input.pad_bytes))),
        default_model: input.default_model.clone(),
        embedding_model: input.embedding_model.clone(),
        max_input_chars: input.max_input_chars,
    };
    let router = match AutoRouter::new(settings, Arc::clone(&recorder), TaskTracker::new()) {
        Ok(router) => router,
        Err(error) => {
            return Ok(ProbeOutput::SemanticRouter(Output {
                accepted: false,
                code: Some(error.code().to_owned()),
                rounds: Vec::new(),
            }));
        }
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime
        .block_on(run_rounds(input, &router, recorder.as_ref()))
        .map_err(|message| invalid(&message))?;
    Ok(ProbeOutput::SemanticRouter(output))
}

fn vectors(
    raw: &IndexMap<String, Vec<Json>>,
) -> Result<IndexMap<String, Vec<f64>>, OperationError> {
    let mut vectors = IndexMap::new();
    for (text, values) in raw {
        let mut vector = Vec::with_capacity(values.len());
        for value in values {
            let Json::Number(number) = value else {
                return Err(invalid("a probe vector holds a value that is not a number"));
            };
            let Some(number) = number.as_f64() else {
                return Err(invalid("a probe vector holds a value that is not a number"));
            };
            vector.push(number);
        }
        vectors.insert(text.clone(), vector);
    }
    Ok(vectors)
}

async fn run_rounds(
    input: &Input,
    router: &AutoRouter<Recorder>,
    recorder: &Recorder,
) -> Result<Output, String> {
    let request = caller_request(input);
    let mut rounds = Vec::new();
    for _round in 0..input.rounds {
        rounds.push(run_round(input, router, recorder, &request).await);
    }
    Ok(Output {
        accepted: true,
        code: None,
        rounds,
    })
}

fn caller_request(input: &Input) -> CallerRequest<'_> {
    CallerRequest {
        metadata: input.metadata.as_ref(),
        litellm_metadata: input.litellm_metadata.as_ref(),
        litellm_params_metadata: input.litellm_params_metadata.as_ref(),
        litellm_session_id: input.litellm_session_id.as_ref(),
        litellm_trace_id: input.litellm_trace_id.as_ref(),
        turn_off_message_logging: input.turn_off_message_logging.as_ref(),
    }
}

async fn run_round(
    input: &Input,
    router: &AutoRouter<Recorder>,
    recorder: &Recorder,
    request: &CallerRequest<'_>,
) -> Round {
    let (documents, queries) = call_counts(recorder);
    let calls = (0..input.concurrent).map(|_caller| {
        decide(
            router,
            Some(&input.messages),
            request,
            PrivacyContext::default(),
        )
    });
    let outcomes = join_all(calls).await;
    let failed = outcomes.iter().any(|outcome| outcome.is_err());
    let model = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref().ok()?.as_ref())
        .map(|decision| decision.model.clone());
    Round {
        failed,
        model: if failed { None } else { model },
        documents: delta(&recorder.documents, documents),
        queries: delta(&recorder.queries, queries),
    }
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
