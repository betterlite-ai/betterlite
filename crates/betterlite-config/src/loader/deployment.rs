use crate::loader::{ConfigError, Lookup, number, parse_float, parse_integer};
use crate::raw::{RawDeployment, RawInfo, RawNumber, RawParams};
use crate::resolve::{resolve_secret, resolve_string};
use crate::types::{LiteLlmParams, ModelDeployment, ModelInfo};

pub(super) fn convert_deployment(
    raw: RawDeployment,
    index: usize,
    lookup: Lookup<'_>,
) -> Result<ModelDeployment, ConfigError> {
    let litellm_params = convert_params(raw.litellm_params, index, lookup)?;
    let model_info = convert_info(raw.model_info, index, lookup)?;
    let model_name = resolve_string(
        raw.model_name,
        &format!("model_list.{index}.model_name"),
        lookup,
    )?;
    Ok(ModelDeployment {
        model_name,
        litellm_params,
        model_info,
    })
}

fn convert_params(
    raw: RawParams,
    index: usize,
    lookup: Lookup<'_>,
) -> Result<LiteLlmParams, ConfigError> {
    let prefix = format!("model_list.{index}.litellm_params");
    let optional = |value: Option<String>, path: &str| {
        value
            .map(|text| resolve_string(text, path, lookup))
            .transpose()
    };
    let model = optional(raw.model, &format!("{prefix}.model"))?;
    let api_base = optional(raw.api_base, &format!("{prefix}.api_base"))?;
    let api_key = resolve_secret(raw.api_key, &format!("{prefix}.api_key"), lookup)?;
    let api_version = optional(raw.api_version, &format!("{prefix}.api_version"))?;
    let float = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_float);
    let integer = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_integer);
    let timeout = float(raw.timeout, &format!("{prefix}.timeout"))?;
    let stream_timeout = float(raw.stream_timeout, &format!("{prefix}.stream_timeout"))?;
    let max_retries = integer(raw.max_retries, &format!("{prefix}.max_retries"))?;
    let rpm = integer(raw.rpm, &format!("{prefix}.rpm"))?;
    let tpm = integer(raw.tpm, &format!("{prefix}.tpm"))?;
    let weight = integer(raw.weight, &format!("{prefix}.weight"))?;
    Ok(LiteLlmParams {
        ssl_verify: raw.ssl_verify,
        model,
        api_base,
        api_key,
        api_version,
        timeout,
        stream_timeout,
        max_retries,
        rpm,
        tpm,
        weight,
    })
}

fn convert_info(raw: RawInfo, index: usize, lookup: Lookup<'_>) -> Result<ModelInfo, ConfigError> {
    let id = raw
        .id
        .map(|text| resolve_string(text, &format!("model_list.{index}.model_info.id"), lookup))
        .transpose()?;
    Ok(ModelInfo { id })
}
