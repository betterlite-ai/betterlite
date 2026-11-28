mod fixture;
mod logging;
mod wire;

use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::{
    ChatConfigTag, ConfigConstruction, ConfigSelection,
};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::builtin::Message;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig as _, EnvironmentRequest, MapCohereRerankParams, Params,
};
use betterlite_provider::model_hub_dispatch::credentials::Globals;
use betterlite_provider::model_hub_dispatch::embedding::{EmbeddingCall, embed};
use betterlite_provider::model_hub_dispatch::hub::headers_from;
use betterlite_provider::model_hub_dispatch::rerank::{
    HuggingFaceRerankConfig, huggingface_rerank_config,
};
use betterlite_provider::model_hub_dispatch::{
    ChatContext, DispatchDeps, HubCompletion, HubDispatchError, dispatch,
};
use betterlite_provider::prepared::PreparedRequest;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use fixture::{Fixture, Recorder, Wire};
use logging::{log_env, log_state};
use wire::{answer, error_status, helper_error};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    HuggingfaceChat,
    HuggingfaceEmbedding,
    HuggingfaceRerank,
    Replicate,
    Predibase,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    op: Operation,
    model: String,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    input_type: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<SecretString>,
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    replies: Vec<Reply>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    status: u16,
    body: String,
}

// the Hugging Face arm only prepares the request, so its wire form is what it prepared
fn keep_prepared(recorder: &Recorder, completion: Option<HubCompletion>) {
    let Some(HubCompletion::HuggingFace(request)) = completion else {
        return;
    };
    let (PreparedRequest::Chat(prepared) | PreparedRequest::Stream(prepared)) = *request else {
        return;
    };
    recorder.keep(Wire {
        method: prepared.http.method.to_string(),
        url: prepared.http.url.to_string(),
        headers: headers_from(&prepared.http.headers),
        body: Some(prepared.http.body),
    });
}

// the chat stream flag and the embedding input_type the case asks for
fn options(case: &Case) -> Object {
    let mut options = Object::new();
    if case.stream {
        options.insert(String::from("stream"), PromptJson::Bool(true));
    }
    if let Some(input_type) = &case.input_type {
        options.insert(
            String::from("input_type"),
            PromptJson::Text(input_type.clone()),
        );
    }
    options
}

// None runs the embedding arm; a tag runs that chat arm through dispatch
fn run_arm(
    case: &Case,
    chat: Option<ChatConfigTag>,
    deps: &DispatchDeps<'_>,
    log: &mut LogContext<'_, '_>,
    runtime: &Runtime,
) -> Result<Option<HubCompletion>, HubDispatchError> {
    let mut asked = options(case);
    let Some(tag) = chat else {
        let embedding = EmbeddingCall {
            model: &case.model,
            input: &case.input,
            optional_params: &asked,
            api_key: case.api_key.as_ref(),
            api_base: case.api_base.as_deref(),
            headers: &BTreeMap::new(),
        };
        return runtime
            .block_on(embed(&embedding, deps, log))
            .map(|_reply| None);
    };
    let mut messages = case.messages.clone();
    let mut call = ChatContext {
        model: &case.model,
        messages: &mut messages,
        optional_params: &mut asked,
        api_key: case.api_key.as_ref(),
        api_base: case.api_base.as_deref(),
        headers: None,
        custom_prompt_dict: None,
        acompletion: false,
    };
    let selection = ConfigSelection {
        tag,
        construction: ConfigConstruction::Shared,
    };
    runtime.block_on(dispatch(&selection, &mut call, deps, log))
}

// litellm.rerank hands the config query, documents and its return_documents=True default
fn cohere_params(
    case: &Case,
    config: &HuggingFaceRerankConfig,
) -> Result<Params, HubDispatchError> {
    let query = case.query.clone().unwrap_or_default();
    let documents = Json::List(case.input.iter().cloned().map(Json::Text).collect());
    let mut cohere = Params::from([
        (String::from("query"), Json::Text(query.clone())),
        (String::from("documents"), documents),
        (String::from("return_documents"), Json::Bool(true)),
    ]);
    let mapped = config.map_cohere_rerank_params(MapCohereRerankParams {
        non_default_params: &mut cohere,
        model: &case.model,
        drop_params: false,
        query: &query,
        documents: &[],
        custom_llm_provider: Some("huggingface"),
        top_n: None,
        rank_fields: None,
        return_documents: Some(true),
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
    })?;
    Ok(mapped)
}

fn rerank(case: &Case, recorder: &Recorder, globals: &Globals<'_>) -> Result<(), HubDispatchError> {
    let config = huggingface_rerank_config(None, None, globals, &case.env)?;
    let mapped = cohere_params(case, &config)?;
    let request = EnvironmentRequest {
        headers: &BTreeMap::new(),
        model: &case.model,
        api_key: None,
        optional_params: None,
        litellm_params: None,
    };
    let headers = config.validate_environment(request)?;
    let body = config.transform_rerank_request(&case.model, &mapped, &headers, None)?;
    let encoded =
        serde_json::to_vec(&body).map_err(|source| HubDispatchError::Encode { source })?;
    recorder.keep(Wire {
        method: String::from("POST"),
        url: config.get_complete_url(None, &case.model, None),
        headers,
        body: Some(encoded),
    });
    Ok(())
}

fn call(
    case: &Case,
    fixture: &Fixture,
    log: &mut LogContext<'_, '_>,
    runtime: &Runtime,
) -> Result<(), HubDispatchError> {
    let globals = Globals::default();
    let deps = fixture.deps(&globals, &case.env);
    match case.op {
        Operation::HuggingfaceChat => {
            let chat = Some(ChatConfigTag::HuggingFaceChat);
            let completion = run_arm(case, chat, &deps, log, runtime)?;
            keep_prepared(&fixture.recorder, completion);
            Ok(())
        }
        Operation::HuggingfaceEmbedding => run_arm(case, None, &deps, log, runtime).map(drop),
        Operation::HuggingfaceRerank => rerank(case, &fixture.recorder, &globals),
        Operation::Replicate => {
            run_arm(case, Some(ChatConfigTag::Replicate), &deps, log, runtime).map(drop)
        }
        Operation::Predibase => {
            run_arm(case, Some(ChatConfigTag::Predibase), &deps, log, runtime).map(drop)
        }
    }
}

const fn provider(op: Operation) -> &'static str {
    match op {
        Operation::HuggingfaceChat
        | Operation::HuggingfaceEmbedding
        | Operation::HuggingfaceRerank => "huggingface",
        Operation::Replicate => "replicate",
        Operation::Predibase => "predibase",
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| helper_error("RuntimeError", &error))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| helper_error("RedactorError", &error))?;
        let fixture = Fixture::new(&input.replies);
        let mut state = log_state(&redactor)?;
        let env = log_env(&redactor);
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let error = call(input, &fixture, &mut log, &runtime).map_or_else(
            |failure| error_status(&failure, provider(input.op), &redactor),
            |()| Value::Null,
        );
        Ok(answer(&fixture.recorder, &error))
    }
}
