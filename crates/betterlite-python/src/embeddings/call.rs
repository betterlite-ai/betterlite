use super::convert::{EmbeddingArguments, EmbeddingCallWire, EmbeddingSettingsWire};
use super::error::{CallFacts, CallFailure, Raised, operation_failure, raised, resolution_failure};
use super::request;
use super::response::{
    EmbeddingAnswer, HiddenFacts, hidden_params, mock_usage, record_json, response_cost,
};
use super::session::{
    DeploymentInputs, EmbeddingSession, additional_headers, deployment, header_map, mock_api_base,
    secret_text, with_headers,
};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::embedding::{EmbeddingRequest, EmbeddingResponse};
use betterlite_provider::operation::{
    EmbeddingOperation, OperationError, OperationRequest, OperationResult,
};
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use std::collections::BTreeMap;
use std::time::Instant;
const MOCK_ERROR: &str = "Mock error";
const MOCK_PROMPT_TOKENS: u32 = 10;
// aembedding stamps the provider on every answer it awaits, a mock answer included
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Sync,
    Async,
}
// One call's borrowed inputs, so each step takes the call instead of its parts.
struct CallContext<'call> {
    session: &'call EmbeddingSession,
    facts: CallFacts<'call>,
    arguments: &'call EmbeddingArguments,
    settings: &'call EmbeddingSettingsWire,
    globals: &'call GlobalCredentials,
    started: Instant,
    entry: Entry,
}
// main.py's embedding call, from the resolved deployment to the answer Python turns into an EmbeddingResponse.
pub(crate) async fn call(
    session: &EmbeddingSession,
    wire: EmbeddingCallWire,
    entry: Entry,
) -> Result<EmbeddingAnswer, CallFailure> {
    let started = Instant::now();
    let EmbeddingCallWire {
        arguments,
        settings,
    } = wire;
    let globals = global_credentials(&settings);
    let api_key = secret_text(arguments.api_key.as_ref())
        .or_else(|| secret_text(settings.api_key.as_ref()))
        .or_else(|| secret_text(settings.openai_key.as_ref()));
    let resolved = session
        .resolve(
            &arguments.model,
            arguments
                .custom_llm_provider
                .as_deref()
                .filter(|text| !text.is_empty()),
            arguments
                .api_base
                .as_deref()
                .filter(|text| !text.is_empty()),
            api_key.as_ref(),
            &globals,
        )
        .map_err(|error| resolution_failure(&error, &arguments.model))?;
    let provider = resolved.provider.slug().to_owned();
    let model = resolved
        .model
        .strip_prefix(&format!("{provider}/"))
        .map_or_else(|| resolved.model.clone(), str::to_owned);
    let context = CallContext {
        session,
        facts: CallFacts {
            call_id: arguments.litellm_call_id.as_deref().unwrap_or_default(),
            model: &model,
            provider: &provider,
        },
        arguments: &arguments,
        settings: &settings,
        globals: &globals,
        started,
        entry,
    };
    answer(&context, &resolved, api_key).await
}
// main.py builds the optional params before mock_response short-circuits, so a refused param fails a mock call too
async fn answer(
    context: &CallContext<'_>,
    resolved: &ResolvedModel,
    api_key: Option<SecretString>,
) -> Result<EmbeddingAnswer, CallFailure> {
    let request = request::build(
        context.arguments,
        context.facts.model,
        context.facts.provider,
        context.settings.drop_params.unwrap_or(false),
        &context.session.environment,
    )?;
    if let Some(mock) = &context.arguments.mock_response {
        return mock_answer(context, mock);
    }
    let inputs = DeploymentInputs {
        resolved,
        facts: &context.facts,
        api_key,
        arguments: context.arguments,
        globals: context.globals,
    };
    let resolved_deployment = deployment(context.session, inputs)?;
    provider_answer(context, &resolved_deployment, request).await
}
// prepare, send and decode through the adapter the resolved deployment names
async fn provider_answer(
    context: &CallContext<'_>,
    resolved: &ChatDeployment,
    request: EmbeddingRequest,
) -> Result<EmbeddingAnswer, CallFailure> {
    let facts = &context.facts;
    let failed = |error: OperationError| operation_failure(error, facts, &context.session.redactor);
    let inputs = Deployment::from_chat(resolved);
    let adapter = Provider::from_deployment(resolved).map_err(failed)?;
    let operation = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: inputs.model.clone(),
        request,
    }));
    let unsent = adapter
        .prepare(&operation, &inputs, &context.session.capabilities)
        .map_err(failed)?;
    let prepared = with_headers(unsent, context.arguments.headers.as_ref())?;
    let sent = Instant::now();
    let upstream = adapter
        .execute(&context.session.client, &prepared, &inputs)
        .await
        .map_err(failed)?;
    let provider_ms = sent.elapsed().as_secs_f64().mul_add(1000.0_f64, 0.0_f64);
    let response_headers = header_map(upstream.headers());
    let decoded = adapter.decode(&prepared, upstream).map_err(failed)?;
    let OperationResult::Embedding(response) = decoded else {
        return Err(raised(
            Raised::InternalServer,
            String::from("the provider answered a different operation than the one sent"),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        ));
    };
    real_answer(context, resolved, provider_ms, response_headers, *response)
}
// mock_embedding answers before the provider runs, with none of a real answer's headers or overhead
fn mock_answer(context: &CallContext<'_>, mock: &Json) -> Result<EmbeddingAnswer, CallFailure> {
    if matches!(mock, Json::Text(text) if text == "error") {
        return Err(raised(Raised::Plain, String::from(MOCK_ERROR), None, None));
    }
    let facts = &context.facts;
    let response_ms = context
        .started
        .elapsed()
        .as_secs_f64()
        .mul_add(1000.0_f64, 0.0_f64);
    let api_base = mock_api_base(
        context.arguments,
        context.settings,
        &context.session.environment,
    );
    let cost = response_cost(
        context.arguments,
        &context.session.catalog,
        facts.model,
        facts.provider,
        MOCK_PROMPT_TOKENS,
    );
    let hidden_facts = HiddenFacts {
        call_id: facts.call_id,
        api_base: &api_base,
        litellm_model: facts.model,
        provider: (context.entry == Entry::Async).then_some(facts.provider),
    };
    let hidden = hidden_params(
        &hidden_facts,
        cost,
        AdditionalHeaders::new(),
        response_ms,
        None,
    );
    Ok(EmbeddingAnswer {
        object: String::from("list"),
        model: String::from(facts.model),
        data: vec![mock_record(mock)],
        usage: mock_usage(),
        hidden_params: hidden,
        response_headers: BTreeMap::new(),
        response_ms,
    })
}
fn mock_record(mock: &Json) -> Json {
    let mut record = IndexMap::new();
    record.insert(
        String::from("object"),
        Json::Text(String::from("embedding")),
    );
    record.insert(String::from("index"), Json::Number(Number::from(0_u32)));
    record.insert(String::from("embedding"), mock.clone());
    Json::Object(record)
}
fn real_answer(
    context: &CallContext<'_>,
    deployment: &ChatDeployment,
    provider_ms: f64,
    response_headers: BTreeMap<String, String>,
    response: EmbeddingResponse,
) -> Result<EmbeddingAnswer, CallFailure> {
    let facts = &context.facts;
    let response_ms = context
        .started
        .elapsed()
        .as_secs_f64()
        .mul_add(1000.0_f64, 0.0_f64);
    let overhead = provider_ms.mul_add(-1.0_f64, response_ms).max(0.0_f64);
    let cost = response_cost(
        context.arguments,
        &context.session.catalog,
        facts.model,
        facts.provider,
        response.usage.prompt_tokens,
    );
    let hidden_facts = HiddenFacts {
        call_id: facts.call_id,
        api_base: &deployment.api_base,
        litellm_model: facts.model,
        provider: Some(facts.provider),
    };
    let headers = additional_headers(&response_headers);
    let hidden = hidden_params(&hidden_facts, cost, headers, response_ms, Some(overhead));
    let usage = serde_json::to_string(&response.usage)
        .and_then(|text| serde_json::from_str::<Json>(&text))
        .map_err(|error| CallFailure::Invalid(error.to_string()))?;
    Ok(EmbeddingAnswer {
        object: response.object,
        model: response.model,
        data: response.data.iter().map(record_json).collect(),
        usage,
        hidden_params: hidden,
        response_headers,
        response_ms,
    })
}
// main.py reads litellm.api_key and litellm.api_base into every call's globals
fn global_credentials(settings: &EmbeddingSettingsWire) -> GlobalCredentials {
    GlobalCredentials {
        api_key: secret_text(settings.api_key.as_ref()),
        api_base: settings.api_base.clone().filter(|text| !text.is_empty()),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}
