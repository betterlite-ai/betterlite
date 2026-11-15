use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_rate_limit::limiter_records::{ModelGroupResolver, RateLimitedModel};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

struct GroupMap(BTreeMap<String, String>);

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

#[derive(Debug, Deserialize)]
struct RatedModelInput {
    #[serde(default)]
    requested: Option<String>,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
struct DescriptorInput {
    auth: BTreeMap<String, JsonData>,
    #[serde(default)]
    requested: Option<String>,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
struct ModelInput {
    requested: String,
    group: String,
}

#[derive(Debug, Deserialize)]
struct RequestMetadataInput {
    #[serde(default)]
    request_metadata: BTreeMap<String, JsonData>,
    #[serde(default)]
    model: Option<ModelInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RatedModel(RatedModelInput),
    Organization(DescriptorInput),
    KeyModel(DescriptorInput),
    TeamModel(DescriptorInput),
    ProjectModel(DescriptorInput),
    ProjectIo(DescriptorInput),
    KeyOwnsTpm(RequestMetadataInput),
}

fn organization_answer(input: &DescriptorInput) -> crate::helpers::HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.auth.clone())).or_value_error()?;
    let descriptors = ModelDescriptorBuilder {
        window_size: input.window_size,
        resolver: Box::new(GroupMap(input.groups.clone())),
    }
    .organization_descriptors(&auth, input.requested.as_deref());
    Ok(json!({"descriptors": descriptors}))
}

fn key_model_answer(input: &DescriptorInput) -> crate::helpers::HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.auth.clone())).or_value_error()?;
    let mut descriptors = Vec::new();
    ModelDescriptorBuilder {
        window_size: input.window_size,
        resolver: Box::new(GroupMap(input.groups.clone())),
    }
    .key_model_descriptor(&auth, input.requested.as_deref(), &mut descriptors)
    .or_value_error()?;
    Ok(json!({"descriptors": descriptors}))
}

fn team_model_answer(input: &DescriptorInput) -> crate::helpers::HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.auth.clone())).or_value_error()?;
    let mut descriptors = Vec::new();
    ModelDescriptorBuilder {
        window_size: input.window_size,
        resolver: Box::new(GroupMap(input.groups.clone())),
    }
    .team_model_descriptor(&auth, input.requested.as_deref(), &mut descriptors);
    Ok(json!({"descriptors": descriptors}))
}

fn project_model_answer(input: &DescriptorInput) -> crate::helpers::HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.auth.clone())).or_value_error()?;
    let mut descriptors = Vec::new();
    ModelDescriptorBuilder {
        window_size: input.window_size,
        resolver: Box::new(GroupMap(input.groups.clone())),
    }
    .project_model_descriptor(&auth, input.requested.as_deref(), &mut descriptors);
    Ok(json!({"descriptors": descriptors}))
}

fn project_io_answer(input: &DescriptorInput) -> crate::helpers::HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.auth.clone())).or_value_error()?;
    let mut descriptors = Vec::new();
    ModelDescriptorBuilder {
        window_size: input.window_size,
        resolver: Box::new(GroupMap(input.groups.clone())),
    }
    .project_io_descriptors(&auth, input.requested.as_deref(), &mut descriptors);
    Ok(json!({"descriptors": descriptors}))
}

fn key_owns_tpm_answer(input: &RequestMetadataInput) -> crate::helpers::HelperResult {
    let model = input.model.as_ref().map(|entry| RateLimitedModel {
        requested: entry.requested.clone(),
        group: entry.group.clone(),
    });
    // ast-grep-ignore: no-long-names
    let owned = ModelDescriptorBuilder::key_owns_tpm_from_request_metadata(
        &input.request_metadata,
        model.as_ref(),
    )
    .or_value_error()?;
    Ok(json!({"owned": owned}))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::RatedModel(case) => {
                let model = ModelDescriptorBuilder {
                    window_size: case.window_size,
                    resolver: Box::new(GroupMap(case.groups.clone())),
                }
                .rated_model(case.requested.as_deref());
                Ok(json!({"model": model.map(|entry| json!({
                    "requested": entry.requested,
                    "group": entry.group,
                }))}))
            }
            Input::Organization(case) => organization_answer(case),
            Input::KeyModel(case) => key_model_answer(case),
            Input::TeamModel(case) => team_model_answer(case),
            Input::ProjectModel(case) => project_model_answer(case),
            Input::ProjectIo(case) => project_io_answer(case),
            Input::KeyOwnsTpm(case) => key_owns_tpm_answer(case),
        }
    }
}
