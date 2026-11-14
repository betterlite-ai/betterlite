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

use super::{Data, FIXED_TOKEN, Settings, guardrail_of, refusal};
use crate::helpers::HelperError;

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

// The Python probe sends the same request, with an empty guardrails list unless the case sets one
fn with_empty_guardrails(data: &Data) -> Data {
    let mut kwargs = BTreeMap::from([("guardrails".to_owned(), GuardrailValue::Array(Vec::new()))]);
    kwargs.extend(data.clone());
    kwargs
}

fn hook_failure(error: DeploymentHookError) -> Result<Value, Box<HelperError>> {
    match error {
        DeploymentHookError::Selection(GuardrailSelectionError::TagMode { .. }) => {
            Ok(json!("rejected"))
        }
        other => Err(refusal("DeploymentHookError", other.to_string())),
    }
}

pub(super) async fn pre_hook(
    settings: &Settings,
    kwargs: &Data,
    call_type: Option<CallType>,
    suppressed: &[String],
) -> Result<Value, Box<HelperError>> {
    let token = PreCallToken(FIXED_TOKEN.to_owned());
    let names: BTreeSet<String> = suppressed.iter().cloned().collect();
    let context = PreCallContext {
        token: &token,
        suppressed: &names,
    };
    let outcome = guardrail_of(settings)
        .pre_call_deployment_hook(&Probe::default(), &context, kwargs.clone(), call_type)
        .await;
    match outcome {
        Ok(result) => Ok(json!(result)),
        Err(error) => hook_failure(error),
    }
}

pub(super) async fn post_hook(
    settings: &Settings,
    request_data: &Data,
    suppressed: &[String],
) -> Result<Value, Box<HelperError>> {
    let names: BTreeSet<String> = suppressed.iter().cloned().collect();
    let mut data = request_data.clone();
    let outcome = guardrail_of(settings)
        .post_call_deployment_hook(
            &Probe::default(),
            &names,
            &mut data,
            GuardrailValue::Object(BTreeMap::new()),
        )
        .await;
    match outcome {
        Ok(result) => Ok(Value::Bool(result.is_some())),
        Err(error) => hook_failure(error),
    }
}

// Runs the pre-call hook the way the Python probe does and gives back every key set the target was handed
async fn probed_keys(
    settings: &Settings,
    data: &Data,
) -> Result<Vec<StampedKeyFields>, Box<HelperError>> {
    let token = PreCallToken(FIXED_TOKEN.to_owned());
    let names = BTreeSet::new();
    let context = PreCallContext {
        token: &token,
        suppressed: &names,
    };
    let probe = Probe::default();
    guardrail_of(settings)
        .pre_call_deployment_hook(
            &probe,
            &context,
            with_empty_guardrails(data),
            Some(CallType::Completion),
        )
        .await
        .map_err(|error| refusal("DeploymentHookError", error.to_string()))?;
    let keys = probe.keys.lock().await.clone();
    Ok(keys)
}

pub(super) async fn already_ran(
    settings: &Settings,
    data: &Data,
) -> Result<Value, Box<HelperError>> {
    let keys = probed_keys(settings, data).await?;
    Ok(Value::Bool(keys.is_empty()))
}

pub(super) async fn stamped(settings: &Settings, data: &Data) -> Result<Value, Box<HelperError>> {
    let keys = probed_keys(settings, data).await?;
    let [key] = keys.as_slice() else {
        return Err(refusal(
            "InputError",
            "the hook did not run once".to_owned(),
        ));
    };
    Ok(json!({
        "user_id": key.user_id,
        "team_id": key.team_id,
        "end_user_id": key.end_user_id,
        "api_key": key.api_key,
        "request_route": key.request_route,
    }))
}
