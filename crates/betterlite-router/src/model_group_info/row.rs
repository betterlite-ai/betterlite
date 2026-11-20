use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_group_info::cost::cost_value;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::fields::{
    auth_field, bool_value, constructor_params, int_value, json_float, mode_value, number_value,
    replacement_params, text_list,
};
use crate::model_group_info::group::{Capability, ClientsideAuth, ModelGroupInfo};
use crate::model_group_info::sources::ProviderAnswer;

// LiteLLM_Params as _set_model_group_info reads it: the model, its provider declaration and the auth params.
pub struct RowParams<'a> {
    pub model: &'a str,
    pub custom_llm_provider: Option<&'a str>,
    pub clientside_auth: Option<Vec<ClientsideAuth>>,
}

impl<'a> RowParams<'a> {
    pub fn parse(params: &'a JsonObject) -> Result<Self, GroupInfoError> {
        let Some(Json::Text(model)) = params.get("model") else {
            return Err(GroupInfoError::Field { field: "model" });
        };
        let custom_llm_provider = match params.get("custom_llm_provider") {
            None | Some(Json::Null) => None,
            Some(Json::Text(provider)) => Some(provider.as_str()),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(GroupInfoError::Field {
                    field: "custom_llm_provider",
                });
            }
        };
        Ok(Self {
            model: model.as_str(),
            custom_llm_provider,
            clientside_auth: auth_field(params, true)?,
        })
    }
}

// The tpm/rpm/itpm/otpm a deployment configures, from the row, then litellm_params, then model_info.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct ConfiguredLimits {
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
}

impl ConfiguredLimits {
    pub fn read(row: &DeploymentSpec) -> Result<Self, GroupInfoError> {
        Ok(Self {
            tpm: first_limit(row, "tpm")?,
            rpm: first_limit(row, "rpm")?,
            itpm: first_limit(row, "itpm")?,
            otpm: first_limit(row, "otpm")?,
        })
    }
}

fn first_limit(row: &DeploymentSpec, field: &'static str) -> Result<Option<i64>, GroupInfoError> {
    for source in [&row.extra, &row.litellm_params, &row.model_info] {
        match source.get(field) {
            None | Some(Json::Null) => {}
            Some(_) => return int_value(source, field),
        }
    }
    Ok(None)
}

// The ModelMapInfo _set_model_group_info builds when the deployment metadata lookup came back empty.
pub fn fallback_metadata(
    model_group: &str,
    provider: &str,
    row: &DeploymentSpec,
    supported: Vec<String>,
) -> Result<JsonObject, GroupInfoError> {
    let mut metadata = JsonObject::new();
    metadata.insert("key".to_owned(), Json::Text(model_group.to_owned()));
    metadata.insert(
        "litellm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    metadata.insert("max_input_tokens".to_owned(), Json::Null);
    metadata.insert("max_output_tokens".to_owned(), Json::Null);
    metadata.insert("mode".to_owned(), fallback_mode(&row.model_info)?);
    let input_cost = cost_value(row.model_info.get("input_cost_per_token"))?;
    metadata.insert(
        "input_cost_per_token".to_owned(),
        json_float(input_cost.as_ref()),
    );
    let output_cost = cost_value(row.model_info.get("output_cost_per_token"))?;
    metadata.insert(
        "output_cost_per_token".to_owned(),
        json_float(output_cost.as_ref()),
    );
    let params = supported
        .iter()
        .map(|name| Json::Text(name.clone()))
        .collect();
    metadata.insert("supported_openai_params".to_owned(), Json::List(params));
    Ok(metadata)
}

fn fallback_mode(model_info: &JsonObject) -> Result<Json, GroupInfoError> {
    match model_info.get("mode") {
        None => Ok(Json::Text("chat".to_owned())),
        Some(Json::Null) => Ok(Json::Null),
        Some(Json::Text(mode)) => Ok(Json::Text(mode.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GroupInfoError::Field { field: "mode" })
        }
    }
}

// ModelGroupInfo(**{"model_group": user_facing, "providers": [provider], **model_info, ...}).
pub fn first_group(
    user_facing: &str,
    answer: &ProviderAnswer,
    metadata: &JsonObject,
) -> Result<ModelGroupInfo, GroupInfoError> {
    Ok(ModelGroupInfo {
        model_group: identity(metadata, user_facing)?,
        providers: providers(metadata, answer)?,
        max_input_tokens: number_value(metadata, "max_input_tokens")?,
        max_output_tokens: number_value(metadata, "max_output_tokens")?,
        input_cost_per_token: number_value(metadata, "input_cost_per_token")?,
        output_cost_per_token: number_value(metadata, "output_cost_per_token")?,
        input_cost_per_pixel: number_value(metadata, "input_cost_per_pixel")?,
        mode: mode_value(metadata)?,
        tpm: int_value(metadata, "tpm")?,
        rpm: int_value(metadata, "rpm")?,
        itpm: int_value(metadata, "itpm")?,
        otpm: int_value(metadata, "otpm")?,
        capabilities: capabilities(metadata)?,
        supports_fast_mode: true,
        supported_reasoning_efforts: None,
        supported_openai_params: constructor_params(metadata)?,
        configurable_clientside_auth_params: auth_field(metadata, false)?,
    })
}

// model_info may override the user-facing model_group the constructor was given.
fn identity(metadata: &JsonObject, user_facing: &str) -> Result<String, GroupInfoError> {
    match metadata.get("model_group") {
        None => Ok(user_facing.to_owned()),
        Some(Json::Text(name)) => Ok(name.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GroupInfoError::Field {
                field: "model_group",
            })
        }
    }
}

fn providers(
    metadata: &JsonObject,
    answer: &ProviderAnswer,
) -> Result<Vec<String>, GroupInfoError> {
    match metadata.get("providers") {
        None => Ok(vec![answer.provider.clone()]),
        Some(Json::Null) => Err(GroupInfoError::Field { field: "providers" }),
        Some(value) => text_list(value, "providers"),
    }
}

// Every flag that reads true, for the constructor and for a later deployment it only ever adds to.
pub fn capabilities(metadata: &JsonObject) -> Result<BTreeSet<Capability>, GroupInfoError> {
    let mut enabled = BTreeSet::new();
    for capability in Capability::ALL {
        if bool_value(metadata, capability.field())? {
            enabled.insert(capability);
        }
    }
    Ok(enabled)
}

// The later branch: every widest value, every true flag and the last supported_openai_params win, and no tpm leaks.
pub fn merge_later(
    group: &mut ModelGroupInfo,
    answer: &ProviderAnswer,
    metadata: &JsonObject,
    limits: &mut ConfiguredLimits,
) -> Result<(), GroupInfoError> {
    if !group
        .providers
        .iter()
        .any(|known| known == &answer.provider)
    {
        group.providers.push(answer.provider.clone());
    }
    widen(
        &mut group.max_input_tokens,
        number_value(metadata, "max_input_tokens")?,
    );
    widen(
        &mut group.max_output_tokens,
        number_value(metadata, "max_output_tokens")?,
    );
    widen(
        &mut group.input_cost_per_token,
        cost_value(metadata.get("input_cost_per_token"))?,
    );
    widen(
        &mut group.output_cost_per_token,
        cost_value(metadata.get("output_cost_per_token"))?,
    );
    group.capabilities.extend(capabilities(metadata)?);
    if let Some(params) = replacement_params(metadata)? {
        group.supported_openai_params = Some(params);
    }
    metadata_limits(limits, metadata)?;
    Ok(())
}

fn widen(current: &mut Option<Decimal>, found: Option<Decimal>) {
    let Some(value) = found else {
        return;
    };
    if current.is_none_or(|held| value > held) {
        *current = Some(value);
    }
}

// A later deployment's own tpm/rpm answer fills a gap the row did not configure, and only then.
fn metadata_limits(
    limits: &mut ConfiguredLimits,
    metadata: &JsonObject,
) -> Result<(), GroupInfoError> {
    if limits.tpm.is_none() {
        limits.tpm = int_value(metadata, "tpm")?;
    }
    if limits.rpm.is_none() {
        limits.rpm = int_value(metadata, "rpm")?;
    }
    Ok(())
}
