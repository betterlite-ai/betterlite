use std::fmt::Display;

use betterlite_bedrock::bedrock_converse_tools_beta::{
    ToolsAndBetaRequest, process_tools_and_beta,
};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::bedrock_tools::capabilities::BedrockToolCapabilities;
use serde_json::json;
use tokio::runtime::Builder;

use super::fixture::Fixture;
use super::input::Input;

fn failed<E>(error: E, status: Option<u16>) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockConverseToolsBeta".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn run(input: &Input) -> HelperResult {
    let fixture = Fixture::load(&input.model_cost).map_err(|error| failed(&error, None))?;
    let resolution = fixture.resolution();
    let dynamic = NoDynamicMetadata;
    let mut keys = ModelCostKeys::default();
    let catalogs = fixture.catalogs();
    let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
    let mut params = input.params.clone();
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| failed(&error, None))?;
    let outcome = runtime
        .block_on(process_tools_and_beta(
            ToolsAndBetaRequest {
                tools: &input.tools,
                model: &input.model,
                headers: input.headers.as_ref(),
                params: &mut params,
                capabilities: BedrockToolCapabilities::default(),
            },
            &mut environment,
            &catalogs,
        ))
        .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({
        "tools": outcome.tools,
        "betas": outcome.betas,
        "params": params,
    }))
}
