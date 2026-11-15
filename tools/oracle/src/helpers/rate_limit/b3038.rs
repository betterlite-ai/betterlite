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

#[derive(Clone, Copy)]
enum DescriptorFunction {
    Organization,
    KeyModel,
    TeamModel,
    ProjectModel,
    ProjectIo,
}

fn descriptors_answer(
    function: DescriptorFunction,
    input: &DescriptorInput,
) -> crate::helpers::HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.auth.clone())).or_value_error()?;
    let builder = ModelDescriptorBuilder {
        window_size: input.window_size,
        resolver: Box::new(GroupMap(input.groups.clone())),
    };
    let requested = input.requested.as_deref();
    let mut descriptors = Vec::new();
    match function {
        DescriptorFunction::Organization => {
            descriptors = builder.organization_descriptors(&auth, requested);
        }
        DescriptorFunction::KeyModel => builder
            .key_model_descriptor(&auth, requested, &mut descriptors)
            .or_value_error()?,
        DescriptorFunction::TeamModel => {
            builder.team_model_descriptor(&auth, requested, &mut descriptors);
        }
        DescriptorFunction::ProjectModel => {
            builder.project_model_descriptor(&auth, requested, &mut descriptors);
        }
        DescriptorFunction::ProjectIo => {
            builder.project_io_descriptors(&auth, requested, &mut descriptors);
        }
    }
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
            Input::Organization(case) => descriptors_answer(DescriptorFunction::Organization, case),
            Input::KeyModel(case) => descriptors_answer(DescriptorFunction::KeyModel, case),
            Input::TeamModel(case) => descriptors_answer(DescriptorFunction::TeamModel, case),
            Input::ProjectModel(case) => descriptors_answer(DescriptorFunction::ProjectModel, case),
            Input::ProjectIo(case) => descriptors_answer(DescriptorFunction::ProjectIo, case),
            Input::KeyOwnsTpm(case) => key_owns_tpm_answer(case),
        }
    }
}
