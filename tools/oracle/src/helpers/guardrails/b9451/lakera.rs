use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::Role;
use betterlite_config::sections::guardrail_identity::item::{GuardrailItem, GuardrailValue};
use betterlite_config::sections::legacy_guardrails::Settings as LegacySettings;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::lakera::{
    LegacyLakeraModeration, LegacyLakeraModerationParams, ModerationCheck,
};
use betterlite_guardrails::third_party_hooks_a::lakera::{
    LakeraGate, LakeraHook, LakeraSelection, LegacyLakeraHooks,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use super::scripted::{Scripted, outcome_view};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;

const HEADER_NAMES: [&str; 1] = ["content-type"];
const ITEM_NAME: &str = "prompt_injection";

#[derive(Debug, Deserialize)]
pub(super) struct Case {
    hook: LakeraHook,
    gate: Option<bool>,
    api_key: String,
    moderation_check: ModerationCheck,
    enabled_roles: Option<Vec<Role>>,
    add_function_to_prompt: bool,
    data: BTreeMap<String, GuardrailValue>,
    answers: Vec<UpstreamAnswer>,
}

pub(super) async fn run(case: &Case) -> HelperResult {
    let scripted = Scripted::start(&case.answers)?;
    let moderation = LegacyLakeraModeration::new(
        LegacyLakeraModerationParams {
            moderation_check: Some(case.moderation_check),
            category_thresholds: None,
            api_base: Some(scripted.base_url.clone()),
            api_key: Some(SecretString::from(case.api_key.clone())),
        },
        |_name| String::new(),
        reqwest::Client::default,
    );
    let mut by_name = BTreeMap::new();
    if let Some(roles) = &case.enabled_roles {
        by_name.insert(
            ITEM_NAME.to_owned(),
            GuardrailItem {
                callbacks: Vec::new(),
                default_on: false,
                logging_only: None,
                guardrail_name: ITEM_NAME.to_owned(),
                callback_args: BTreeMap::new(),
                enabled_roles: Some(roles.clone()),
            },
        );
    }
    let legacy = LegacyLakeraHooks::new(
        moderation,
        &LegacySettings {
            items: Vec::new(),
            by_name,
        },
        case.add_function_to_prompt,
    );
    let gate = case.gate.map_or(LakeraGate::Legacy, LakeraGate::Checked);
    // the caller's B3665 gate with no dynamic params, and the B3663 metadata answer that lets the check proceed
    let base = GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    };
    let selection = LakeraSelection {
        should_proceed: true,
        dynamic_params: &base,
    };
    let outcome = legacy
        .hook(case.hook, &case.data, gate, &selection)
        .await
        .map(|()| json!({"completed": true}));
    let secrets = [("authorization", format!("Bearer {}", case.api_key))];
    Ok(outcome_view(
        &scripted.take_requests()?,
        &HEADER_NAMES,
        &secrets,
        &outcome,
    ))
}
