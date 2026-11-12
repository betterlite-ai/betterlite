use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_provider::bedrock_dispatch_contract::{
    BedrockAwsSettings, BedrockDispatchError, BedrockOperation, BedrockPlanRequest,
    BedrockPrepared, dispatch,
};
use indexmap::IndexSet;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperResult;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    price_map: ParamMap,
    #[serde(default)]
    converse_models: Vec<String>,
    #[serde(default)]
    request_metadata_fields: Vec<String>,
    cases: Vec<PlanCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanCase {
    model: String,
    model_id: Option<String>,
    aws_region_name: Option<String>,
    api_base: Option<String>,
    aws_bedrock_runtime_endpoint: Option<String>,
    #[serde(default)]
    settings: BedrockAwsSettings,
    #[serde(default)]
    request_params: ParamMap,
    #[serde(default)]
    additional_drop_params: Vec<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let converse_models: IndexSet<String> = input.converse_models.iter().cloned().collect();
        let plan = json!({
            "plans": input
                .cases
                .iter()
                .map(|case| case_plan(case, input, &converse_models))
                .collect::<Vec<Value>>(),
        });
        Ok(plan)
    }
}

fn case_plan(case: &PlanCase, input: &Input, converse_models: &IndexSet<String>) -> Value {
    let request = BedrockPlanRequest {
        model: case.model.clone(),
        model_id: case.model_id.clone(),
        aws_region_name: case.aws_region_name.clone(),
        api_base: case.api_base.clone(),
        aws_bedrock_runtime_endpoint: case.aws_bedrock_runtime_endpoint.clone(),
        settings: case.settings.clone(),
        request_params: case.request_params.clone(),
        additional_drop_params: case.additional_drop_params.clone(),
        price_map: input.price_map.clone(),
        converse_models: converse_models.clone(),
        request_metadata_owned: !input.request_metadata_fields.is_empty(),
    };
    plan_value(dispatch(&BedrockOperation::Plan(Box::new(request))))
}

fn plan_value(result: Result<BedrockPrepared, BedrockDispatchError>) -> Value {
    match result {
        Ok(BedrockPrepared::Invoke {
            model,
            model_id,
            region,
            endpoint_url,
            proxy_endpoint_url,
        }) => json!({"plan": {
            "operation": "invoke",
            "model": model,
            "model_id": model_id,
            "region": region,
            "endpoint_url": endpoint_url,
            "proxy_endpoint_url": proxy_endpoint_url,
        }}),
        Ok(BedrockPrepared::Routed { operation, model }) => {
            json!({"plan": {"operation": operation, "model": model}})
        }
        Err(BedrockDispatchError::Route(_)) => json!({"error": "route"}),
        Err(BedrockDispatchError::Endpoint(AwsEndpointError::Region { .. })) => {
            json!({"error": "region"})
        }
        Err(
            error @ (BedrockDispatchError::Endpoint(_)
            | BedrockDispatchError::Encode { .. }
            | BedrockDispatchError::Unsupported { .. }),
        ) => json!({"error": error.to_string()}),
    }
}
