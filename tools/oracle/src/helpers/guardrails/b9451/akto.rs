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

use super::scripted::{Scripted, outcome_view};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;

const ENCODED_FIELDS: [&str; 6] = [
    "requestHeaders",
    "responseHeaders",
    "requestPayload",
    "responsePayload",
    "tag",
    "metadata",
];

#[derive(Debug, Deserialize)]
pub(crate) struct Case {
    akto_api_key: String,
    unreachable_fallback: String,
    event_hook: LitellmParamsMode,
    texts: Vec<String>,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    answers: Vec<UpstreamAnswer>,
}

pub(super) async fn run(case: &Case) -> HelperResult {
    let scripted = Scripted::start(&case.answers)?;
    let guardrail = mirroring(case, scripted.base_url.clone())?;
    let mut inputs = AktoInputs::default();
    inputs.texts.clone_from(&case.texts);
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
    let names = ["content-type"];
    Ok(outcome_view(&requests, &names, &secrets, &outcome))
}

fn mirroring(case: &Case, api_base: String) -> Result<AktoMirroring, Box<HelperError>> {
    let configured = Configured {
        guardrail_name: "akto".to_owned(),
        api_base,
        api_key: SecretString::from(case.akto_api_key.clone()),
        account_id: "1000000".to_owned(),
        vxlan_id: "0".to_owned(),
        unreachable_fallback: if case.unreachable_fallback == "fail_open" {
            UnreachableFallback::FailOpen
        } else {
            UnreachableFallback::FailClosed
        },
        timeout: Duration::from_secs(5),
    };
    AktoMirroring::new(configured, reqwest::Client::default()).or_config_error()
}

fn decoded_body(body: &Value) -> Result<Value, Box<HelperError>> {
    let mut fields = body.clone();
    let Some(object) = fields.as_object_mut() else {
        return Err("the Akto request body is not an object").or_config_error();
    };
    object.insert("time".to_owned(), json!("<millis>"));
    for name in ENCODED_FIELDS {
        let text = object
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("the Akto field {name} is not text"))
            .or_config_error()?;
        let mut inner: Value = serde_json::from_str(text).or_config_error()?;
        let inner_text = inner.get("body").and_then(Value::as_str).map(str::to_owned);
        if let (Some(encoded), Some(slot)) = (inner_text, inner.get_mut("body")) {
            *slot = serde_json::from_str(&encoded).or_config_error()?;
        }
        object.insert(name.to_owned(), inner);
    }
    Ok(fields)
}
