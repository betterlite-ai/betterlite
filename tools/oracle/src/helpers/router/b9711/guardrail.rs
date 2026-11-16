use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_startup::router_list::RouterGuardrail;
use betterlite_guardrails::router_guardrail_selection::select_router_guardrail;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct GuardrailInput {
    guardrails: Vec<GuardrailEntry>,
    guardrail_name: String,
}

#[derive(Debug, Deserialize)]
struct GuardrailEntry {
    guardrail_name: String,
    id: Option<String>,
    litellm_params: GuardrailParams,
}

#[derive(Debug, Deserialize)]
struct GuardrailParams {
    guardrail: String,
    mode: LitellmParamsMode,
    api_key: Option<String>,
    api_base: Option<String>,
}

pub(super) fn run(input: &GuardrailInput) -> HelperResult {
    let list: Vec<RouterGuardrail> = input
        .guardrails
        .iter()
        .map(|entry| RouterGuardrail {
            guardrail_name: entry.guardrail_name.clone(),
            id: entry.id.clone(),
            guardrail: entry.litellm_params.guardrail.clone(),
            mode: entry.litellm_params.mode.clone(),
            api_key: entry.litellm_params.api_key.clone().map(SecretString::from),
            api_base: entry.litellm_params.api_base.clone(),
        })
        .collect();
    // The cases never hold two entries with the asked name, so nothing is drawn
    let mut rng = StdRng::seed_from_u64(0);
    let picked =
        select_router_guardrail(&list, &input.guardrail_name, &mut rng).or_value_error()?;
    Ok(serde_json::json!({
        "guardrail_name": picked.guardrail_name,
        "id": picked.id,
    }))
}
