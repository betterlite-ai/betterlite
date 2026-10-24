use std::task::{Context, Poll, Waker};

use betterlite_bedrock::request_policy::aws_params::{map_auth_aliases, merge_aws_request_params};
use betterlite_bedrock::request_policy::validation::validate_request_metadata;
use betterlite_bedrock::request_policy::{
    BedrockRequestPolicy, OneResponse, REQUEST_METADATA_HEADER, RequestValue, merge_invoke_headers,
};
use futures_util::{StreamExt as _, stream};
use serde_json::{Value, json};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BedrockRequestPolicyInput {
    #[serde(default)]
    pub(crate) fields: Vec<String>,
    pub(crate) step: PolicyStep,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PolicyStep {
    Resolve {
        litellm_params: Value,
        caller: Value,
    },
    Validate {
        metadata: Vec<Value>,
    },
    Merge {
        headers: Vec<(String, String)>,
        guardrail: Vec<(String, String)>,
        litellm_params: Value,
    },
    Aliases {
        non_default: Value,
        optional: Value,
    },
    MergeParams {
        deployment: Value,
        request: Value,
    },
    Iterate {
        response: Value,
        calls: Vec<IteratorCall>,
    },
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IteratorCall {
    Sync,
    Async,
}

pub(crate) fn invoke(input: &BedrockRequestPolicyInput) -> Result<Value, Box<super::HelperError>> {
    let policy = BedrockRequestPolicy::from_fields(&input.fields);
    let projected = match &input.step {
        PolicyStep::Resolve { litellm_params, caller } => {
            let params = entries(litellm_params);
            let caller_value = request_value(caller);
            let resolved = policy.as_ref().and_then(|owner| owner.resolve(&params, Some(&caller_value)));
            let header = policy.as_ref().and_then(|owner| owner.metadata_header(&params));
            let names: Vec<String> = policy.iter().map(|_| REQUEST_METADATA_HEADER.to_lowercase()).collect();
            let pairs: Vec<(&str, String)> = header.map(|value| (REQUEST_METADATA_HEADER, value)).into_iter().collect();
            json!({"owned": policy.is_some(), "resolved": resolved, "header_names": names, "header_pairs": pairs})
        }
        PolicyStep::Validate { metadata } => metadata
            .iter()
            .map(|value| match validate_request_metadata(&request_value(value)) {
                Ok(()) => Value::Null,
                Err(error) => json!({"class": "BadRequestError", "status": error.status_code(), "message": error.to_string()}),
            })
            .collect(),
        PolicyStep::Merge { headers, guardrail, litellm_params } => {
            json!(merge_invoke_headers(headers, guardrail, policy.as_ref(), &entries(litellm_params)))
        }
        PolicyStep::Aliases { non_default, optional } => {
            let mut params = entries(optional);
            map_auth_aliases(&entries(non_default), &mut params);
            pairs_json(&params)?
        }
        PolicyStep::MergeParams { deployment, request } => {
            pairs_json(&merge_aws_request_params(&entries(deployment), &entries(request)))?
        }
        PolicyStep::Iterate { response, calls } => iterate(response, calls),
    };
    Ok(projected)
}

fn iterate(response: &Value, calls: &[IteratorCall]) -> Value {
    let mut responses: OneResponse<&Value> = Some(response).into_iter();
    let mut context = Context::from_waker(Waker::noop());
    let mut outcomes = Vec::new();
    for call in calls {
        let next = match call {
            IteratorCall::Sync => Poll::Ready(responses.next()),
            IteratorCall::Async => stream::iter(&mut responses).poll_next_unpin(&mut context),
        };
        outcomes.push(match next {
            Poll::Ready(Some(value)) => json!({"yield": value}),
            Poll::Ready(None) => json!("stop"),
            Poll::Pending => json!("pending"),
        });
    }
    Value::Array(outcomes)
}

fn request_value(value: &Value) -> RequestValue {
    match value {
        Value::String(text) => RequestValue::Text(text.clone()),
        Value::Object(object) => RequestValue::Mapping(
            object
                .iter()
                .map(|(key, entry)| (key.clone(), request_value(entry)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
            RequestValue::Raw(value.to_string())
        }
    }
}

fn entries(value: &Value) -> Vec<(String, RequestValue)> {
    match request_value(value) {
        RequestValue::Mapping(pairs) => pairs,
        RequestValue::Text(_) | RequestValue::Secret(_) | RequestValue::Raw(_) => Vec::new(),
    }
}

fn pairs_json(params: &[(String, RequestValue)]) -> Result<Value, Box<super::HelperError>> {
    let mut pairs = Vec::new();
    for (key, value) in params {
        pairs.push(json!([key, json_value(value)?]));
    }
    Ok(Value::Array(pairs))
}

fn json_value(value: &RequestValue) -> Result<Value, Box<super::HelperError>> {
    match value {
        RequestValue::Text(text) => Ok(Value::String(text.clone())),
        // request_value never builds a secret, so nothing reaches this arm
        RequestValue::Secret(_) => Ok(Value::Null),
        RequestValue::Mapping(pairs) => {
            let mut object = serde_json::Map::new();
            for (key, entry) in pairs {
                object.insert(key.clone(), json_value(entry)?);
            }
            Ok(Value::Object(object))
        }
        RequestValue::Raw(raw) => serde_json::from_str(raw).map_err(|error| {
            Box::new(super::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        }),
    }
}

mod tests;
