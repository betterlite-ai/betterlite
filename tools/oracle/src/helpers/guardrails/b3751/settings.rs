use betterlite_config::loader::ConfigLoader;
use betterlite_guardrails::akto::AktoMirroring;

use super::Input;
use super::SettingsInput;
use super::support::failure;

pub(super) fn build(input: &Input) -> Result<AktoMirroring, Box<crate::helpers::HelperError>> {
    let loader = ConfigLoader {
        lookup: input.env.clone(),
    };
    let config = loader
        .load(&source(&input.settings))
        .map_err(|error| failure("ConfigError", None, error.to_string()))?;
    let configured = config
        .sections
        .akto_mirroring
        .guardrails
        .first()
        .ok_or_else(|| failure("ConfigError", None, "no akto guardrail entry".to_owned()))?
        .clone();
    AktoMirroring::new(configured, reqwest::Client::default())
        .map_err(|error| failure("AktoError", None, error.to_string()))
}

fn source(settings: &SettingsInput) -> String {
    let mut parameters = vec!["guardrail: akto".to_owned()];
    if let Some(value) = &settings.akto_base_url {
        parameters.push(format!("akto_base_url: {value:?}"));
    }
    if let Some(value) = &settings.akto_api_key {
        parameters.push(format!("akto_api_key: {value:?}"));
    }
    if let Some(value) = &settings.akto_account_id {
        parameters.push(format!("akto_account_id: {value:?}"));
    }
    if let Some(value) = &settings.akto_vxlan_id {
        parameters.push(format!("akto_vxlan_id: {value:?}"));
    }
    if let Some(value) = &settings.unreachable_fallback {
        parameters.push(format!("unreachable_fallback: {value:?}"));
    }
    if let Some(value) = settings.guardrail_timeout {
        parameters.push(format!("guardrail_timeout: {value}"));
    }
    format!(
        "guardrails:\n  - guardrail_name: akto\n    litellm_params:\n      {}\n",
        parameters.join("\n      ")
    )
}
