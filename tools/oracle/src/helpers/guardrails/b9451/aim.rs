use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aim::settings::{AimGuardrail, AimParams};
use betterlite_guardrails::aim::transport::{AimWebSocketConnector, HttpPoster};
use betterlite_guardrails::lasso_masking::wire::ModelResponseShape;
use betterlite_guardrails::third_party_hooks_a::aim::{self, AimRequestHook};
use betterlite_guardrails::third_party_hooks_a::error::ThirdPartyHookError;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::scripted::{Hook, Scripted, chat_response, contents, failure, outcome_view};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;

const HEADER_NAMES: [&str; 4] = [
    "content-type",
    "x-aim-call-id",
    "x-aim-gateway-key-alias",
    "x-aim-litellm-hook",
];

#[derive(Debug, Deserialize)]
pub(super) struct Case {
    hook: Hook,
    api_key: String,
    inspect_embeddings: bool,
    call_type: String,
    data: BTreeMap<String, GuardrailValue>,
    outputs: Vec<String>,
    key_alias: Option<String>,
    answers: Vec<UpstreamAnswer>,
}

pub(super) async fn run(case: &Case) -> HelperResult {
    let scripted = Scripted::start(&case.answers)?;
    let params = AimParams {
        guardrail_name: Some("aim".to_owned()),
        api_key: Some(SecretString::from(case.api_key.clone())),
        api_base: Some(scripted.base_url.clone()),
        inspect_embeddings: Some(case.inspect_embeddings),
        litellm_version: None,
    };
    let poster = HttpPoster {
        client: reqwest::Client::default(),
    };
    let guardrail = AimGuardrail::new(
        params,
        &BTreeMap::<String, String>::new(),
        Arc::new(poster),
        Arc::new(AimWebSocketConnector),
        Some(Duration::from_secs(30)),
    )
    .map_err(failure)?;
    let mut response = chat_response(&case.outputs)?;
    let outcome = call_hook(&guardrail, case, &mut response).await;
    let secrets = [("authorization", format!("Bearer {}", case.api_key))];
    let requests = scripted.take_requests()?;
    Ok(outcome_view(&requests, &HEADER_NAMES, &secrets, &outcome))
}

async fn call_hook(
    guardrail: &AimGuardrail,
    case: &Case,
    response: &mut ModelResponseShape,
) -> Result<Value, ThirdPartyHookError> {
    let key_alias = case.key_alias.as_deref();
    let mut data = case.data.clone();
    let request_hook = match case.hook {
        Hook::PreCall => AimRequestHook::PreCall,
        Hook::Moderation => AimRequestHook::Moderation,
        Hook::PostCall => {
            aim::post_call_success(guardrail, &data, response, key_alias).await?;
            return Ok(contents(response));
        }
    };
    aim::hook(
        guardrail,
        &mut data,
        request_hook,
        &case.call_type,
        key_alias,
    )
    .await?;
    Ok(match request_hook {
        AimRequestHook::PreCall => json!({"data": data}),
        AimRequestHook::Moderation => json!({"returns_input": data == case.data}),
    })
}
