use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;

// B8666's dispatch builds the class, so these folders keep the params under the variant their wire name picks
const CLASS_KINDS: &[(&str, fn(Box<LitellmParams>) -> HookKind)] = &[
    (wire::BLOCK_CODE_EXECUTION, HookKind::BlockCodeExecution),
    (wire::COMPRESR, HookKind::Compresr),
    (wire::CUSTOM_CODE, HookKind::CustomCode),
    (wire::HIDDENLAYER, HookKind::Hiddenlayer),
    (wire::JAVELIN, HookKind::Javelin),
    (wire::LLM_AS_A_JUDGE, HookKind::LlmAsAJudge),
    (wire::NOMA, HookKind::Noma),
    (wire::NOMA_V2, HookKind::NomaV2),
    (wire::ONYX, HookKind::Onyx),
    (wire::OVALIX, HookKind::Ovalix),
    (wire::PANGEA, HookKind::Pangea),
    (wire::REPELLOAI, HookKind::Repelloai),
];

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    _name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    CLASS_KINDS
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, make)| Ok(HookSpec::new(settings, make(Box::new(params.clone())))))
}
