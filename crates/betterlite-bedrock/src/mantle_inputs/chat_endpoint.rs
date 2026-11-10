use secrecy::SecretString;

use crate::aws_endpoints::region::validate_region;
use crate::model_routes::ParamMap;

use super::capability::base_segment;
use super::credentials::resolve_bearer_token;
use super::error::MantleInputError;
use super::records::MantleGlobals;
use super::region::{DEFAULT_REGION, split_region_prefix};

#[derive(Debug, Clone)]
pub struct ChatEndpointInputs {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChatEndpointParams<'text> {
    pub model: Option<&'text str>,
    pub api_base: Option<&'text str>,
    pub api_key: Option<&'text SecretString>,
    pub aws_region_name: Option<&'text str>,
}

pub fn chat_endpoint_inputs(
    params: ChatEndpointParams<'_>,
    globals: &MantleGlobals,
    model_cost: &ParamMap,
) -> Result<ChatEndpointInputs, MantleInputError> {
    let region = selected_region(params, globals);
    validate_region(Some(&region))?;
    let (_, base_model) = split_model(params.model);
    let api_base = selected_api_base(params.api_base, globals, &region, base_model, model_cost);
    let api_key = resolve_bearer_token(
        params.api_key,
        globals.mantle_api_key.as_ref(),
        globals.aws_bearer_token.as_ref(),
    );
    Ok(ChatEndpointInputs { api_base, api_key })
}

fn split_model(model: Option<&str>) -> (Option<&str>, Option<&str>) {
    let Some(name) = model.filter(|value| !value.is_empty()) else {
        return (None, None);
    };
    let (region, base) = split_region_prefix(name);
    (region, Some(base))
}

fn selected_region(params: ChatEndpointParams<'_>, globals: &MantleGlobals) -> String {
    let (prefix_region, _) = split_model(params.model);
    let region = [
        params.aws_region_name.filter(|value| !value.is_empty()),
        prefix_region,
        globals
            .mantle_region
            .as_deref()
            .filter(|value| !value.is_empty()),
        globals
            .aws_region_name
            .as_deref()
            .filter(|value| !value.is_empty()),
        globals
            .aws_region
            .as_deref()
            .filter(|value| !value.is_empty()),
    ]
    .into_iter()
    .flatten()
    .next()
    .unwrap_or(DEFAULT_REGION);
    region.to_owned()
}

fn selected_api_base(
    api_base: Option<&str>,
    globals: &MantleGlobals,
    region: &str,
    base_model: Option<&str>,
    model_cost: &ParamMap,
) -> String {
    let base = [
        api_base.filter(|value| !value.is_empty()),
        globals
            .mantle_api_base
            .as_deref()
            .filter(|value| !value.is_empty()),
    ]
    .into_iter()
    .flatten()
    .next();
    base.map_or_else(
        || {
            format!(
                "https://bedrock-mantle.{region}.api.aws/{}",
                base_segment(base_model, model_cost)
            )
        },
        str::to_owned,
    )
}
