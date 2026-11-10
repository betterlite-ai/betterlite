use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};

use crate::mantle_inputs::records::MantleGlobals;
use crate::model_routes::{ParamMap, ParamValue};

const FIXTURE_CONFIG: &str =
    "model_list:\n  - model_name: fixture\n    litellm_params: {model: bedrock/fixture}\n";

pub(super) fn globals(environment: &[(&str, &str)]) -> MantleGlobals {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(FIXTURE_CONFIG).unwrap();
    let mantle = &config.sections.bedrock_mantle_inputs;
    let aws = &config.sections.bedrock_aws_endpoints;
    MantleGlobals {
        mantle_api_key: mantle.api_key.clone(),
        mantle_api_base: mantle.api_base.clone(),
        mantle_region: mantle.region.clone(),
        aws_bearer_token: config
            .sections
            .bedrock_settings
            .deployments
            .first()
            .and_then(|deployment| deployment.bearer_token.clone()),
        aws_region_name: aws.aws_region_name.clone(),
        aws_region: aws.aws_region.clone(),
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

pub(super) fn text(value: &str) -> ParamValue {
    ParamValue::Text(value.to_owned())
}

pub(super) fn list(values: &[&str]) -> ParamValue {
    ParamValue::List(values.iter().map(|value| text(value)).collect())
}

pub(super) fn entry(fields: &[(&str, ParamValue)]) -> ParamValue {
    ParamValue::Mapping(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<IndexMap<_, _>>(),
    )
}

pub(super) fn cost(entries: &[(&str, ParamValue)]) -> ParamMap {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect::<IndexMap<_, _>>()
}

pub(super) fn empty_cost() -> ParamMap {
    IndexMap::new()
}
