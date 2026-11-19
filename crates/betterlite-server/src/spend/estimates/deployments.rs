use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use rust_decimal::Decimal;

use super::error::EstimateError;

// What calculate_spend reads from the deployment it bills: its underlying model and its token overrides.
pub struct MatchedDeployment<'spec> {
    pub model: Option<&'spec str>,
    pub custom_cost_per_token: Option<CustomCostPerToken>,
}

// The alias a request name rewrites to, which the search compares against model_name.
fn search_name<'name>(aliases: &'name AliasMap, requested: &'name str) -> Option<&'name str> {
    aliases
        .get(requested)
        .map_or(Some(requested), |alias| alias.resolved())
}

// calculate_spend's lookup: the alias resolves first and the last listed deployment with that model_name wins.
#[must_use]
pub fn find_deployment<'list>(
    deployments: &'list [DeploymentSpec],
    aliases: &AliasMap,
    requested: &str,
) -> Option<&'list DeploymentSpec> {
    let name = search_name(aliases, requested)?;
    deployments
        .iter()
        .rfind(|deployment| deployment.model_name == name)
}

// The matched deployment's billing model and the only custom pricing the route honors: its token overrides.
pub fn matched_deployment(
    deployment: &DeploymentSpec,
) -> Result<MatchedDeployment<'_>, EstimateError> {
    Ok(MatchedDeployment {
        model: text_model(&deployment.litellm_params),
        custom_cost_per_token: custom_cost_per_token(&deployment.litellm_params)?,
    })
}

// A non-string model reaches cost_per_token as None and raises there; a string, empty included, is billed as given.
fn text_model(params: &JsonObject) -> Option<&str> {
    match params.get("model") {
        Some(Json::Text(model)) => Some(model.as_str()),
        None | Some(_) => None,
    }
}

enum Rate {
    Absent,
    Value(Decimal),
    Invalid,
}

fn configured_rate(params: &JsonObject, name: &str) -> Rate {
    match params.get(name) {
        None | Some(Json::Null) => Rate::Absent,
        Some(Json::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).map_or(Rate::Invalid, Rate::Value)
        }
        Some(_) => Rate::Invalid,
    }
}

// CostPerToken takes both rates together, and cost_per_token raises TypeError when one is None.
fn custom_cost_per_token(params: &JsonObject) -> Result<Option<CustomCostPerToken>, EstimateError> {
    let input = configured_rate(params, "input_cost_per_token");
    let output = configured_rate(params, "output_cost_per_token");
    match (input, output) {
        (Rate::Absent, Rate::Absent) => Ok(None),
        (Rate::Value(input_cost_per_token), Rate::Value(output_cost_per_token)) => {
            Ok(Some(CustomCostPerToken {
                input_cost_per_token,
                output_cost_per_token,
                cache_read_input_token_cost: None,
                cache_creation_cost: None,
            }))
        }
        _ => Err(EstimateError::Failure {
            message: "the deployment must set both input_cost_per_token and output_cost_per_token for custom pricing".to_owned(),
        }),
    }
}
