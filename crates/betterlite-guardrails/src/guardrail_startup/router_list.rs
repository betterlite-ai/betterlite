use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use secrecy::SecretString;

// api_key is the secret itself after the os.environ/ replacement, so it stays a SecretString
#[derive(Debug, Clone)]
pub struct RouterGuardrail {
    pub guardrail_name: String,
    pub id: Option<String>,
    pub guardrail: String,
    pub mode: LitellmParamsMode,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

#[must_use]
pub fn router_guardrail_list(guardrails: &[Guardrail]) -> Vec<RouterGuardrail> {
    let list: Vec<RouterGuardrail> = guardrails
        .iter()
        .map(|guardrail| RouterGuardrail {
            guardrail_name: guardrail.guardrail_name.clone(),
            id: guardrail.guardrail_id.clone(),
            guardrail: guardrail.litellm_params.guardrail.clone(),
            mode: guardrail.litellm_params.mode.clone(),
            api_key: guardrail.litellm_params.base.fields.api_key.clone(),
            api_base: guardrail.litellm_params.base.fields.api_base.clone(),
        })
        .collect();
    tracing::debug!(
        "Populated router guardrail_list with {} guardrails",
        list.len()
    );
    list
}
