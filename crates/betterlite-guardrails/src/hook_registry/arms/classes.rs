use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;

// B8666's dispatch builds the class, so these arms keep the params under the variant their wire name picks
pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    _name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let class = match kind {
        wire::BLOCK_CODE_EXECUTION => HookKind::BlockCodeExecution(Box::new(params.clone())),
        wire::COMPRESR => HookKind::Compresr(Box::new(params.clone())),
        wire::CUSTOM_CODE => HookKind::CustomCode(Box::new(params.clone())),
        wire::HIDDENLAYER => HookKind::Hiddenlayer(Box::new(params.clone())),
        wire::JAVELIN => HookKind::Javelin(Box::new(params.clone())),
        wire::LLM_AS_A_JUDGE => HookKind::LlmAsAJudge(Box::new(params.clone())),
        wire::NOMA => HookKind::Noma(Box::new(params.clone())),
        wire::NOMA_V2 => HookKind::NomaV2(Box::new(params.clone())),
        wire::ONYX => HookKind::Onyx(Box::new(params.clone())),
        wire::OVALIX => HookKind::Ovalix(Box::new(params.clone())),
        wire::PANGEA => HookKind::Pangea(Box::new(params.clone())),
        wire::REPELLOAI => HookKind::Repelloai(Box::new(params.clone())),
        _other => return None,
    };
    Some(Ok(HookSpec::new(settings, class)))
}
