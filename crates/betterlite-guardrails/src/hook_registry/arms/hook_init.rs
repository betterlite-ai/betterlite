use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;
use crate::hook_initializers::azure::initialize_azure;
use crate::hook_initializers::ibm::initialize_ibm;
use crate::hook_initializers::model_armor::initialize_model_armor;
use crate::hook_initializers::openai::initialize_openai;
use crate::hook_initializers::purview::initialize_purview;

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::AZURE_PROMPT_SHIELD => {
            let result = initialize_azure(params, Some(name));
            arms.take(result, HookKind::AzurePromptShield)
        }
        wire::AZURE_TEXT_MODERATIONS => {
            let result = initialize_azure(params, Some(name));
            arms.take(result, HookKind::AzureTextModerations)
        }
        wire::IBM_GUARDRAILS => {
            arms.take(initialize_ibm(params, Some(name)), HookKind::IbmGuardrails)
        }
        wire::MICROSOFT_PURVIEW => arms.take(
            initialize_purview(params, Some(name)),
            HookKind::MicrosoftPurview,
        ),
        wire::MODEL_ARMOR => Ok(arms.plain(
            initialize_model_armor(params, Some(name)),
            HookKind::ModelArmor,
        )),
        wire::OPENAI_MODERATION => arms.take(
            initialize_openai(params, Some(name)),
            HookKind::OpenaiModeration,
        ),
        _other => return None,
    })
}
