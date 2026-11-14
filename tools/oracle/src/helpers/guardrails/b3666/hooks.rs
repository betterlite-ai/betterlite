use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_deployment::hook_target::{
    DeploymentHookTarget, HookTarget, PreCallOutcome, StampedKeyFields,
};
use betterlite_guardrails::guardrail_deployment::hooks::{DeploymentHookError, PreCallContext};
use betterlite_guardrails::guardrail_deployment::markers::PreCallToken;
use betterlite_guardrails::guardrail_selection::error::GuardrailSelectionError;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::{Data, FIXED_TOKEN, Settings, guardrail_of};
use crate::helpers::{HelperError, OrHelperError as _};

// Both sides run the same hook and note the key fields it was handed
#[derive(Default)]
struct Probe {
    keys: Mutex<Vec<StampedKeyFields>>,
}

impl DeploymentHookTarget for Probe {
    async fn pre_call(
        &self,
        _target: HookTarget,
        key: &StampedKeyFields,
        _data: &mut Data,
        _call_type: CallType,
    ) -> Result<PreCallOutcome, GuardrailFailure> {
        self.keys.lock().await.push(key.clone());
        let message = GuardrailValue::Object(BTreeMap::from([
            ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            (
                "content".to_owned(),
                GuardrailValue::Text("hooked".to_owned()),
            ),
        ]));
        Ok(PreCallOutcome::Data(BTreeMap::from([(
            "messages".to_owned(),
            GuardrailValue::Array(vec![message]),
        )])))
    }

    async fn post_call_success(
        &self,
        _target: HookTarget,
        _key: &StampedKeyFields,
        _data: &mut Data,
        _response: GuardrailValue,
    ) -> Result<Option<GuardrailValue>, GuardrailFailure> {
        Ok(Some(GuardrailValue::Object(BTreeMap::new())))
    }
}

// A tag-mode rejection is an answer both sides give; any other failure is a refusal
fn verdict<T, F>(
    outcome: Result<T, DeploymentHookError>,
    shape: F,
) -> Result<Value, Box<HelperError>>
where
    F: FnOnce(T) -> Value,
{
    match outcome {
        Err(DeploymentHookError::Selection(GuardrailSelectionError::TagMode { .. })) => {
            Ok(json!("rejected"))
        }
        other => other.map(shape).or_value_error(),
    }
}

pub(super) async fn pre_hook(
    settings: &Settings,
    kwargs: &Data,
    call_type: Option<CallType>,
    suppressed: &BTreeSet<String>,
) -> Result<Value, Box<HelperError>> {
    let token = PreCallToken(FIXED_TOKEN.to_owned());
    let context = PreCallContext {
        token: &token,
        suppressed,
    };
    let outcome = guardrail_of(settings)
        .pre_call_deployment_hook(&Probe::default(), &context, kwargs.clone(), call_type)
        .await;
    verdict(outcome, |result| json!(result))
}

pub(super) async fn post_hook(
    settings: &Settings,
    request_data: &Data,
    suppressed: &BTreeSet<String>,
) -> Result<Value, Box<HelperError>> {
    let mut data = request_data.clone();
    let outcome = guardrail_of(settings)
        .post_call_deployment_hook(
            &Probe::default(),
            suppressed,
            &mut data,
            GuardrailValue::Object(BTreeMap::new()),
        )
        .await;
    verdict(outcome, |result| Value::Bool(result.is_some()))
}

// Runs the pre-call hook as the Python probe does, with an empty guardrails list unless the case sets one
pub(super) async fn probed_keys(
    settings: &Settings,
    data: &Data,
) -> Result<Vec<StampedKeyFields>, Box<HelperError>> {
    let token = PreCallToken(FIXED_TOKEN.to_owned());
    let context = PreCallContext {
        token: &token,
        suppressed: &BTreeSet::new(),
    };
    let probe = Probe::default();
    let mut kwargs = BTreeMap::from([("guardrails".to_owned(), GuardrailValue::Array(Vec::new()))]);
    kwargs.extend(data.clone());
    guardrail_of(settings)
        .pre_call_deployment_hook(&probe, &context, kwargs, Some(CallType::Completion))
        .await
        .or_value_error()?;
    Ok(probe.keys.into_inner())
}

pub(super) async fn stamped(settings: &Settings, data: &Data) -> Result<Value, Box<HelperError>> {
    let keys = probed_keys(settings, data).await?;
    let [key] = keys.as_slice() else {
        return Err::<Value, _>("the hook did not run once").or_value_error();
    };
    Ok(json!({
        "user_id": key.user_id,
        "team_id": key.team_id,
        "end_user_id": key.end_user_id,
        "api_key": key.api_key_hash,
        "request_route": key.request_route,
    }))
}
