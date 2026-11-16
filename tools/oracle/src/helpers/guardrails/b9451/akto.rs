use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::akto_mirroring::{Configured, UnreachableFallback};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::akto::AktoMirroring;
use betterlite_guardrails::akto::payload::AktoInputs;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::third_party_hooks_a::akto;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::scripted::{Scripted, failure, outcome_view};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;

const HEADER_NAMES: [&str; 1] = ["content-type"];
// the payload fields that hold JSON text, some with a second JSON text under "body"
const ENCODED_FIELDS: [&str; 6] = [
    "requestHeaders",
    "responseHeaders",
    "requestPayload",
    "responsePayload",
    "tag",
    "metadata",
];

#[derive(Debug, Deserialize)]
pub(super) struct Case {
    akto_api_key: String,
    unreachable_fallback: Fallback,
    event_hook: LitellmParamsMode,
    texts: Vec<String>,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    answers: Vec<UpstreamAnswer>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Fallback {
    FailClosed,
    FailOpen,
}

pub(super) async fn run(case: &Case) -> HelperResult {
    let scripted = Scripted::start(&case.answers)?;
    let configured = Configured {
        guardrail_name: "akto".to_owned(),
        api_base: scripted.base_url.clone(),
        api_key: SecretString::from(case.akto_api_key.clone()),
        account_id: "1000000".to_owned(),
        vxlan_id: "0".to_owned(),
        unreachable_fallback: match case.unreachable_fallback {
            Fallback::FailClosed => UnreachableFallback::FailClosed,
            Fallback::FailOpen => UnreachableFallback::FailOpen,
        },
        timeout: Duration::from_secs(5),
    };
    let guardrail = AktoMirroring::new(configured, reqwest::Client::default()).map_err(failure)?;
    let inputs = AktoInputs {
        texts: case.texts.clone(),
        model: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
    };
    let outcome = akto::apply_guardrail(
        &guardrail,
        inputs,
        &case.request_data,
        case.input_type,
        &case.event_hook,
    )
    .await
    .map(|returned| json!({"texts": returned.texts}));
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    let mut requests = scripted.take_requests()?;
    for request in &mut requests {
        request.body = decoded_body(&request.body)?;
    }
    let secrets = [("authorization", case.akto_api_key.clone())];
    Ok(outcome_view(&requests, &HEADER_NAMES, &secrets, &outcome))
}

fn decoded_body(body: &Value) -> Result<Value, Box<HelperError>> {
    let mut fields = body.clone();
    let Some(object) = fields.as_object_mut() else {
        return Err(failure("the Akto request body is not an object"));
    };
    object.insert("time".to_owned(), json!("<millis>"));
    for name in ENCODED_FIELDS {
        let text = object
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| failure(format!("the Akto field {name} is not text")))?;
        let mut inner: Value = serde_json::from_str(text).map_err(failure)?;
        let inner_text = inner.get("body").and_then(Value::as_str).map(str::to_owned);
        if let (Some(encoded), Some(slot)) = (inner_text, inner.get_mut("body")) {
            *slot = serde_json::from_str(&encoded).map_err(failure)?;
        }
        object.insert(name.to_owned(), inner);
    }
    Ok(fields)
}
