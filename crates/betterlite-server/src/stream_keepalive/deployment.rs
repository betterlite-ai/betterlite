use betterlite_config::sections::settings::JsonValue;

use super::{JsonObject, KeepaliveRouter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentKeepalive {
    pub seconds: Option<JsonValue>,
    pub allow_client_override: bool,
}

#[must_use]
pub fn deployment_keepalive(
    router: Option<&dyn KeepaliveRouter>,
    request_data: &JsonObject,
    hidden: &JsonObject,
) -> Option<DeploymentKeepalive> {
    let model_key = hidden
        .get("model_id")
        .and_then(JsonValue::as_str)
        .filter(|model_id| !model_id.is_empty());
    if let Some(model_id) = model_key {
        let params = router?.deployment_params(model_id)?;
        return Some(DeploymentKeepalive {
            seconds: params
                .get("keepalive_seconds")
                .filter(|seconds| !seconds.is_null())
                .cloned(),
            allow_client_override: params.get("allow_client_keepalive_override")
                == Some(&JsonValue::Bool(true)),
        });
    }
    let model_name = request_data.get("model").and_then(JsonValue::as_str);
    let mut distinct: Vec<DeploymentKeepalive> = Vec::new();
    for deployment in router?.group_deployments(model_name) {
        let pair = group_pair(&deployment);
        let agrees = distinct.iter().any(|known| {
            known.allow_client_override == pair.allow_client_override
                && seconds_agree(known.seconds.as_ref(), pair.seconds.as_ref())
        });
        if !agrees {
            distinct.push(pair);
        }
    }
    match distinct.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

fn group_pair(deployment: &JsonValue) -> DeploymentKeepalive {
    let params = deployment
        .get("litellm_params")
        .and_then(JsonValue::as_object);
    let seconds = params
        .and_then(|fields| fields.get("keepalive_seconds"))
        .filter(|seconds| !seconds.is_null())
        .cloned();
    let allow_client_override = params
        .and_then(|fields| fields.get("allow_client_keepalive_override"))
        == Some(&JsonValue::Bool(true));
    DeploymentKeepalive {
        seconds,
        allow_client_override,
    }
}

fn seconds_agree(left: Option<&JsonValue>, right: Option<&JsonValue>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(JsonValue::Number(left_number)), Some(JsonValue::Number(right_number))) => {
            left_number.as_f64() == right_number.as_f64()
        }
        (Some(JsonValue::String(left_text)), Some(JsonValue::String(right_text))) => {
            left_text == right_text
        }
        (Some(left_value), Some(right_value)) => left_value == right_value,
        _ => false,
    }
}
