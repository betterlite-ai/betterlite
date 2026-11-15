use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::HookInitError;
use super::{ParamMap, read, required_name, required_text};

mod tests;

const VENDOR: &str = "IBM Guardrails";

struct OptionalParams {
    detector_params: BTreeMap<String, GuardrailValue>,
    extra_headers: BTreeMap<String, GuardrailValue>,
    score_threshold: Option<f64>,
    block_on_detection: GuardrailFlag,
}

#[derive(Debug, Clone)]
pub struct IbmArgs {
    pub guardrail_name: String,
    pub auth_token: SecretString,
    pub base_url: String,
    pub detector_id: String,
    pub is_detector_server: GuardrailFlag,
    pub detector_params: BTreeMap<String, GuardrailValue>,
    pub extra_headers: BTreeMap<String, GuardrailValue>,
    pub score_threshold: Option<f64>,
    pub block_on_detection: GuardrailFlag,
    pub verify_ssl: GuardrailFlag,
    pub default_on: GuardrailFlag,
    pub event_hook: LitellmParamsMode,
    pub timeout: Option<f64>,
}

pub fn initialize_ibm(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<IbmArgs, HookInitError> {
    let extra = &params.base.extra.0;
    let auth_token = SecretString::from(required_text(extra, VENDOR, "auth_token")?);
    let base_url = required_text(extra, VENDOR, "base_url")?;
    let detector_id = required_text(extra, VENDOR, "detector_id")?;
    let name = required_name(guardrail_name, VENDOR)?;
    let optional = optional_params(extra)?;
    Ok(IbmArgs {
        guardrail_name: name,
        auth_token,
        base_url,
        detector_id,
        is_detector_server: read::<GuardrailFlag>(extra, VENDOR, "is_detector_server")?
            .unwrap_or(true),
        detector_params: optional.detector_params,
        extra_headers: optional.extra_headers,
        score_threshold: optional.score_threshold,
        block_on_detection: optional.block_on_detection,
        verify_ssl: read::<GuardrailFlag>(extra, VENDOR, "verify_ssl")?.unwrap_or(true),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        event_hook: params.mode.clone(),
        timeout: params.base.fields.timeout,
    })
}

fn optional_params(extra: &ParamMap) -> Result<OptionalParams, HookInitError> {
    let Some(optional) = read::<ParamMap>(extra, VENDOR, "optional_params")? else {
        return Ok(OptionalParams {
            detector_params: BTreeMap::new(),
            extra_headers: BTreeMap::new(),
            score_threshold: None,
            block_on_detection: true,
        });
    };
    Ok(OptionalParams {
        detector_params: read::<ParamMap>(&optional, VENDOR, "detector_params")?
            .unwrap_or_default(),
        extra_headers: read::<ParamMap>(&optional, VENDOR, "extra_headers")?.unwrap_or_default(),
        score_threshold: read::<f64>(&optional, VENDOR, "score_threshold")?,
        block_on_detection: read::<GuardrailFlag>(&optional, VENDOR, "block_on_detection")?
            .unwrap_or(true),
    })
}
