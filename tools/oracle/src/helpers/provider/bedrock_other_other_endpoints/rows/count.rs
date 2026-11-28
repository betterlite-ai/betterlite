use betterlite_bedrock::count_call::counter::{
    CountTokensCall, CountTokensDeployment, TokenCountResponse,
};
use betterlite_bedrock::count_call::handler::{BedrockCountTokensHandler, CountTokensEnvironment};
use betterlite_bedrock::transcription::error::CallError;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::{BedrockOtherOperation, CountOperation};
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use betterlite_provider::bedrock_other_dispatch::test_support::client::FixtureClient;
use betterlite_provider::bedrock_other_dispatch::test_support::{NOW_SECONDS, instant};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::harness::{Harness, Outcome, exchange};
use super::super::model::{Failed, Probe, param_map};
use super::row;
use crate::openai_media_dispatch::failure;

pub(super) async fn count_row(probe: &Probe) -> Result<Value, Failed> {
    let harness = Harness::new(probe)?;
    let handler = BedrockCountTokensHandler {
        transport: FixtureClient {
            replies: harness.replies.clone(),
            seen: harness.seen.clone(),
        },
        credentials: harness.source.clone(),
        environment: environment(probe),
    };
    let deployment = CountTokensDeployment {
        litellm_params: param_map(probe)?,
    };
    let request_model = probe
        .request_model
        .clone()
        .unwrap_or_else(|| probe.model.clone());
    let call = CountTokensCall {
        model_to_use: &probe.model,
        messages: probe.messages.as_ref(),
        deployment: Some(&deployment),
        request_model: &request_model,
        tools: None,
        system: None,
        now: instant(NOW_SECONDS).map_err(|error| failure("Instant", error))?,
    };
    let operation = BedrockOtherOperation::Count(CountOperation {
        call,
        handler: Some(handler),
    });
    let outcome: Outcome<CallError> = exchange::<_, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::Count(record)) => count_value(record.as_deref())?,
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, None)
}

// The region, endpoint and bearer token LiteLLM reads from the process environment
fn environment(probe: &Probe) -> CountTokensEnvironment {
    CountTokensEnvironment {
        region_name: probe.environment.get("AWS_REGION_NAME").cloned(),
        region: probe.environment.get("AWS_REGION").cloned(),
        session_region: probe.environment.get("AWS_DEFAULT_REGION").cloned(),
        runtime_endpoint: probe
            .environment
            .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
            .cloned(),
        bearer_token: probe
            .environment
            .get("AWS_BEARER_TOKEN_BEDROCK")
            .cloned()
            .map(SecretString::from),
    }
}

// A failed count stays a record: nothing here falls back to a local tokenizer
fn count_value(record: Option<&TokenCountResponse>) -> Result<Value, Failed> {
    let Some(found) = record else {
        return Ok(Value::Null);
    };
    let original = match &found.original_response {
        None => Value::Null,
        Some(reply) => serde_json::to_value(reply).map_err(|error| failure("Count", error))?,
    };
    Ok(json!({
        "total_tokens": found.total_tokens,
        "request_model": &found.request_model,
        "model_used": &found.model_used,
        "tokenizer_type": &found.tokenizer_type,
        "error": found.error,
        "error_message": &found.error_message,
        "status_code": found.status_code,
        "original_response": original,
    }))
}
