use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::lasso_classification::Configured;
use betterlite_guardrails::lasso::{ConversationCache, LassoGuardrail};
use betterlite_guardrails::lasso_masking::wire::ModelResponseShape;
use betterlite_guardrails::third_party_hooks_a::error::ThirdPartyHookError;
use betterlite_guardrails::third_party_hooks_a::lasso::{self, LassoRequestHook, PostCallResponse};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use ulid::Ulid;

use super::scripted::{Hook, Scripted, chat_response, contents, outcome_view};
use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;

const HEADER_NAMES: [&str; 4] = [
    "content-type",
    "lasso-api-key",
    "lasso-conversation-id",
    "lasso-user-id",
];

#[derive(Debug, Deserialize)]
pub(super) struct Case {
    hook: Hook,
    gate: bool,
    api_key: String,
    conversation_id: String,
    mask: bool,
    timeout: f64,
    data: BTreeMap<String, GuardrailValue>,
    outputs: Vec<String>,
    answers: Vec<UpstreamAnswer>,
}

#[derive(Default)]
struct Cache(Mutex<BTreeMap<String, String>>);

impl ConversationCache for Cache {
    type Error = fmt::Error;

    fn get_cache(&self, key: &str) -> Result<Option<String>, fmt::Error> {
        let entries = self.0.lock().map_err(|_poisoned| fmt::Error)?;
        Ok(entries.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: &str, _ttl: Duration) -> Result<(), fmt::Error> {
        let mut entries = self.0.lock().map_err(|_poisoned| fmt::Error)?;
        entries.insert(key.to_owned(), value.to_owned());
        drop(entries);
        Ok(())
    }
}

pub(super) async fn run(case: &Case) -> HelperResult {
    let scripted = Scripted::start(&case.answers)?;
    let guardrail = LassoGuardrail {
        settings: Configured {
            guardrail_name: "lasso".to_owned(),
            api_key: SecretString::from(case.api_key.clone()),
            api_base: scripted.base_url.clone(),
            user_id: None,
            conversation_id: Some(case.conversation_id.clone()),
            mask: case.mask,
        },
        client: reqwest::Client::default(),
        timeout: Duration::try_from_secs_f64(case.timeout).or_config_error()?,
        new_id: Ulid::generate,
    };
    let mut response = chat_response(&case.outputs)?;
    let outcome = call_hook(&guardrail, case, &mut response).await;
    let requests = scripted.take_requests()?;
    Ok(outcome_view(&requests, &HEADER_NAMES, &[], &outcome))
}

async fn call_hook(
    guardrail: &LassoGuardrail,
    case: &Case,
    response: &mut ModelResponseShape,
) -> Result<Value, ThirdPartyHookError> {
    let cache = Cache::default();
    let mut data = case.data.clone();
    let request_hook = match case.hook {
        Hook::PreCall => LassoRequestHook::PreCall,
        Hook::Moderation => LassoRequestHook::Moderation,
        Hook::PostCall => {
            let chat = PostCallResponse::ChatCompletion(&mut *response);
            lasso::post_call_success(guardrail, &data, chat, &cache, case.gate).await?;
            return Ok(contents(response));
        }
    };
    lasso::hook(guardrail, &mut data, &cache, request_hook, case.gate).await?;
    Ok(json!({"data": data}))
}
