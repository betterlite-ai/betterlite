use betterlite_bedrock::model_routes::ParamMap;
use betterlite_provider::bedrock_dispatch_contract::{
    BedrockAwsSettings, BedrockOperation, BedrockPlanRequest, BedrockPrepared, dispatch,
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
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    aws_region_name: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    aws_bedrock_runtime_endpoint: Option<String>,
    #[serde(default)]
    settings: SettingsCase,
    #[serde(default)]
    request_params: ParamMap,
    #[serde(default)]
    additional_drop_params: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsCase {
    #[serde(default)]
    aws_region_name: Option<String>,
    #[serde(default)]
    aws_region: Option<String>,
    #[serde(default)]
    session_region: Option<String>,
    #[serde(default)]
    aws_bedrock_runtime_endpoint: Option<String>,
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
        settings: BedrockAwsSettings {
            aws_region_name: case.settings.aws_region_name.clone(),
            aws_region: case.settings.aws_region.clone(),
            session_region: case.settings.session_region.clone(),
            aws_bedrock_runtime_endpoint: case.settings.aws_bedrock_runtime_endpoint.clone(),
        },
        request_params: case.request_params.clone(),
        additional_drop_params: case.additional_drop_params.clone(),
        price_map: input.price_map.clone(),
        converse_models: converse_models.clone(),
        request_metadata_owned: !input.request_metadata_fields.is_empty(),
    };
    match dispatch(&BedrockOperation::Plan(Box::new(request))) {
        Ok(prepared) => json!({"plan": prepared_value(&prepared)}),
        Err(_error) => json!({"error": true}),
    }
}

fn prepared_value(prepared: &BedrockPrepared) -> Value {
    json!({
        "operation": prepared.operation,
        "model": prepared.model,
        "model_id": prepared.model_id,
        "region": prepared.region,
        "endpoint_url": prepared.endpoint_url,
        "proxy_endpoint_url": prepared.proxy_endpoint_url,
    })
}
