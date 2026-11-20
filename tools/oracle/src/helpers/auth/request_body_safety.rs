use std::collections::BTreeMap;

use betterlite_auth::request_body_safety::{
    FormMetadataExtractor, ModelGrantLookup, ModelGrants, RequestBodySafety, is_request_body_safe,
};
use betterlite_config::sections::request_body_safety::{
    ClientsideParamGrant, DEFAULT_MAX_FALLBACKS, model_grant_map,
};
use betterlite_core::api::request_form::metadata::extract_nested_form_metadata;
use betterlite_core::api::request_form::values::{FormFields, FormValue};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    request_body: JsonObject,
    general_settings: JsonObject,
    model: String,
    model_list: Vec<DeploymentInput>,
}

#[derive(Debug, Deserialize)]
struct DeploymentInput {
    model_name: String,
    #[serde(default)]
    litellm_params: LiteLlmParamsInput,
}

#[derive(Debug, Default, Deserialize)]
struct LiteLlmParamsInput {
    #[serde(default, rename = "configurable_clientside_auth_params")]
    clientside_params: Option<Vec<Json>>,
}

#[derive(Debug)]
struct Grants(BTreeMap<String, Option<Vec<ClientsideParamGrant>>>);

impl ModelGrantLookup for Grants {
    fn grants(&self, model_group: &str) -> ModelGrants<'_> {
        match self.0.get(model_group) {
            None => ModelGrants::Absent,
            Some(None) => ModelGrants::Unset,
            Some(Some(items)) => ModelGrants::Granted(items),
        }
    }
}

// B1101 builds the bracket-form tree from the request dict's keys, exactly as is_request_body_safe does.
#[derive(Debug)]
struct FormNames;

impl FormMetadataExtractor for FormNames {
    fn top_level_names(&self, form_data: &JsonObject, prefix: &str) -> Vec<String> {
        let fields: FormFields = form_data
            .keys()
            .map(|name| (name.clone(), FormValue::Text(String::new())))
            .collect();
        let (tree, _warnings) = extract_nested_form_metadata(&fields, prefix);
        tree.keys().cloned().collect()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let grants = model_grant_map(input.model_list.iter().map(|deployment| {
            (
                Some(deployment.model_name.clone()),
                deployment.litellm_params.clientside_params.clone(),
            )
        }))
        .map_err(|error| Box::new(helper_error("ValidationError", error.to_string())))?;
        let lookup = Grants(grants);
        let allow_client_side_credentials = matches!(
            input.general_settings.get("allow_client_side_credentials"),
            Some(Json::Bool(true))
        );
        match is_request_body_safe(&RequestBodySafety {
            body: &input.request_body,
            model: &input.model,
            allow_client_side_credentials,
            allowed_hosts: &[],
            grants: &lookup,
            metadata: &FormNames,
            max_fallbacks: DEFAULT_MAX_FALLBACKS,
        }) {
            Ok(()) => Ok(Value::Bool(true)),
            Err(error) => Err(Box::new(helper_error("ValueError", error.to_string()))),
        }
    }
}

// dispatch.py reports every helper exception with null type, param and code.
fn helper_error(class: &str, message: String) -> HelperError {
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    }
}
