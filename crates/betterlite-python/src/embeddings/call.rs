use super::convert::{EmbeddingArguments, EmbeddingCallWire, EmbeddingSettingsWire};
use super::error::{
    CallFacts, CallFailure, internal_server, operation_failure, plain_exception, resolution_failure,
};
use super::request;
use super::response::{
    EmbeddingAnswer, HiddenFacts, hidden_params, milliseconds, mock_usage, response_cost,
    response_data, usage_json,
};
use super::session::{
    EmbeddingSession, additional_headers, deployment, header_map, mock_api_base, plain_text,
    secret_text, with_headers,
};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_provider::operation::{EmbeddingOperation, OperationRequest, OperationResult};
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use indexmap::IndexMap;
use serde_json::Number;
use std::collections::BTreeMap;
use std::time::Instant;
const MOCK_ERROR: &str = "Mock error";
const MOCK_PROMPT_TOKENS: u32 = 10;
struct Timing {
    started: Instant,
    provider_ms: f64,
}
// main.py's embedding call, from the resolved deployment to the answer Python turns into an EmbeddingResponse.
pub(crate) async fn call(
    session: &EmbeddingSession,
    wire: EmbeddingCallWire,
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
            plain_text(arguments.custom_llm_provider.as_deref()).as_deref(),
            plain_text(arguments.api_base.as_deref()).as_deref(),
            api_key.as_ref(),
            &globals,
        )
        .map_err(|error| resolution_failure(&error, &arguments.model))?;
    let provider = resolved.provider.slug().to_owned();
    let prefix = format!("{provider}/");
    let model = resolved
        .model
        .strip_prefix(&prefix)
        .map_or_else(|| resolved.model.clone(), str::to_owned);
    let facts = CallFacts {
        call_id: arguments.litellm_call_id.as_deref().unwrap_or_default(),
        model: &model,
        provider: &provider,
    };
    let request = request::build(
        &arguments,
        &model,
        &provider,
        settings.drop_params.unwrap_or(false),
        &session.environment,
    )?;
    if let Some(mock) = &arguments.mock_response {
        return mock_answer(session, &facts, &arguments, mock, &settings, started);
    }
    let resolved_deployment = deployment(
        session, &resolved, &facts, &prefix, api_key, &arguments, &globals,
    )?;
    let inputs = Deployment::from_chat(&resolved_deployment);
    let adapter = Provider::from_deployment(&resolved_deployment)
        .map_err(|error| operation_failure(error, &facts, &session.redactor))?;
    let operation = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: inputs.model.clone(),
        request,
    }));
    let prepared = adapter
        .prepare(&operation, &inputs, &session.capabilities)
        .map_err(|error| operation_failure(error, &facts, &session.redactor))?;
    let prepared = with_headers(prepared, arguments.headers.as_ref())?;
    let sent = Instant::now();
    let upstream = adapter
        .execute(&session.client, &prepared, &inputs)
        .await
        .map_err(|error| operation_failure(error, &facts, &session.redactor))?;
    let timing = Timing {
        started,
        provider_ms: milliseconds(sent.elapsed()),
    };
    let response_headers = header_map(upstream.headers());
    let decoded = adapter
        .decode(&prepared, upstream)
        .map_err(|error| operation_failure(error, &facts, &session.redactor))?;
    let OperationResult::Embedding(response) = decoded else {
        return Err(internal_server(
            String::from("the provider answered a different operation than the one sent"),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        ));
    };
    real_answer(
        session,
        &facts,
        &arguments,
        &resolved_deployment,
        &timing,
        response_headers,
        *response,
    )
}
// mock_embedding answers before the provider runs, with none of a real answer's headers or overhead
fn mock_answer(
    session: &EmbeddingSession,
    facts: &CallFacts<'_>,
    arguments: &EmbeddingArguments,
    mock: &Json,
    settings: &EmbeddingSettingsWire,
    started: Instant,
) -> Result<EmbeddingAnswer, CallFailure> {
    if matches!(mock, Json::Text(text) if text == "error") {
        return Err(plain_exception(String::from(MOCK_ERROR)));
    }
    let response_ms = milliseconds(started.elapsed());
    let api_base = mock_api_base(arguments, settings, &session.environment);
    let cost = response_cost(
        arguments,
        &session.catalog,
        facts.model,
        facts.provider,
        MOCK_PROMPT_TOKENS,
    );
    let hidden_facts = HiddenFacts {
        call_id: facts.call_id,
        api_base: &api_base,
        litellm_model: facts.model,
        provider: None,
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
    session: &EmbeddingSession,
    facts: &CallFacts<'_>,
    arguments: &EmbeddingArguments,
    deployment: &ChatDeployment,
    timing: &Timing,
    response_headers: BTreeMap<String, String>,
    response: EmbeddingResponse,
) -> Result<EmbeddingAnswer, CallFailure> {
    let response_ms = milliseconds(timing.started.elapsed());
    let overhead = timing
        .provider_ms
        .mul_add(-1.0_f64, response_ms)
        .max(0.0_f64);
    let cost = response_cost(
        arguments,
        &session.catalog,
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
    let hidden = hidden_params(
        &hidden_facts,
        cost,
        additional_headers(&response_headers),
        response_ms,
        Some(overhead),
    );
    let usage =
        usage_json(&response.usage).map_err(|error| CallFailure::Invalid(error.to_string()))?;
    let data = response_data(&response.data);
    Ok(EmbeddingAnswer {
        object: response.object,
        model: response.model,
        data,
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
        api_base: plain_text(settings.api_base.as_deref()),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}
