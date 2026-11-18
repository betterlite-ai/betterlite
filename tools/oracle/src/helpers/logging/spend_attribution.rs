use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::spend_attribution::Settings as SpendAttributionSettings;
use betterlite_logging::spend_metadata::access_groups::{
    AccessGroupSources, request_model_access_groups,
};
use betterlite_logging::spend_metadata::ocr::extract_usage_for_ocr_call;
use betterlite_logging::spend_metadata::projection::{
    SpendMetadataArguments, project_spend_logs_metadata,
};
use betterlite_logging::spend_metadata::redaction::redact_logged_api_key;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::identity::spend_proxy::{
    DeploymentIdentity, ProxySelection, deployment_provider,
};
use betterlite_server::spend::attribution::{configured_model_group, model_group_provider};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Redact(RedactInput),
    Project(ProjectInput),
    AccessGroups(AccessGroupsInput),
    OcrUsage(OcrUsageInput),
    Providers(ProvidersInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RedactInput {
    values: Vec<KeyInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyInput {
    value: Option<String>,
    already_redacted: bool,
    key_alias: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectInput {
    metadata: Option<JsonObject>,
    store_bodies: bool,
    arguments: SpendMetadataArguments,
}

// The auth object cannot travel as JSON, so the groups it carries arrive beside the metadata mappings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccessGroupsInput {
    standard_logging_object: Option<JsonObject>,
    metadata: Option<JsonObject>,
    litellm_metadata: Option<JsonObject>,
    auth_groups: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OcrUsageInput {
    usage_info: Option<Json>,
    attribute_present: bool,
    attribute_usage_info: Json,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvidersInput {
    deployments: Vec<DeploymentInput>,
    model_group: String,
    recognized: Vec<String>,
    team_public: Vec<String>,
    env_proxy: bool,
    global_proxy: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentInput {
    model_name: String,
    litellm_params: JsonObject,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Redact(case) => {
                let values = case
                    .values
                    .iter()
                    .map(|key| {
                        redact_logged_api_key(
                            key.value.as_deref(),
                            key.already_redacted,
                            key.key_alias.as_deref(),
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(json!(values))
            }
            Input::Project(case) => {
                let policy = SpendSanitizePolicy {
                    store_bodies: case.store_bodies,
                    max_string_length: 0,
                };
                let record = project_spend_logs_metadata(
                    case.metadata.as_ref(),
                    case.arguments.clone(),
                    &policy,
                )
                .or_value_error()?;
                serde_json::to_value(record).or_value_error()
            }
            Input::AccessGroups(case) => {
                let mut params = JsonObject::new();
                if let Some(metadata) = &case.metadata {
                    params.insert("metadata".to_owned(), Json::Object(metadata.clone()));
                }
                if let Some(metadata) = &case.litellm_metadata {
                    params.insert(
                        "litellm_metadata".to_owned(),
                        Json::Object(metadata.clone()),
                    );
                }
                let auth_groups = case
                    .auth_groups
                    .as_ref()
                    .map(|groups| Json::List(groups.iter().cloned().map(Json::Text).collect()));
                let groups = request_model_access_groups(&AccessGroupSources {
                    standard_logging_object: case.standard_logging_object.as_ref(),
                    litellm_params: Some(&params),
                    auth_groups: auth_groups.as_ref(),
                });
                serde_json::to_value(groups).or_value_error()
            }
            Input::OcrUsage(case) => serde_json::to_value(extract_usage_for_ocr_call(
                case.usage_info.as_ref(),
                case.attribute_present.then_some(&case.attribute_usage_info),
            ))
            .or_value_error(),
            Input::Providers(case) => providers(case),
        }
    }
}

fn providers(input: &ProvidersInput) -> HelperResult {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let selection = ProxySelection {
        environment: input.env_proxy,
        global: input.global_proxy,
    };
    let identities: Vec<DeploymentIdentity<'_>> = input
        .deployments
        .iter()
        .map(|deployment| DeploymentIdentity::from_litellm_params(&deployment.litellm_params))
        .collect();
    let resolved: Vec<Option<String>> = identities
        .iter()
        .map(|identity| deployment_provider(identity, &selection, &context))
        .collect();
    let matching: Vec<DeploymentIdentity<'_>> = input
        .deployments
        .iter()
        .zip(&identities)
        .filter(|(deployment, _)| deployment.model_name == input.model_group)
        .map(|(_, identity)| *identity)
        .collect();
    let settings = SpendAttributionSettings {
        environment_use_litellm_proxy: input.env_proxy,
        global_use_litellm_proxy: input.global_proxy,
        deployment_use_litellm_proxy: Vec::new(),
    };
    let group_provider = model_group_provider(&input.model_group, &matching, &settings, &context);
    let recognized = input
        .recognized
        .iter()
        .cloned()
        .collect::<BTreeSet<String>>();
    let team_public = input
        .team_public
        .iter()
        .cloned()
        .collect::<BTreeSet<String>>();
    let configured = configured_model_group(&input.model_group, &recognized, &team_public);
    Ok(json!({
        "providers": resolved,
        "group_provider": group_provider,
        "configured": configured,
    }))
}
