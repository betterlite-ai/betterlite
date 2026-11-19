use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;

// These folders' initializers build a class from ports only B8666's dispatch has
pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    _name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::BLOCK_CODE_EXECUTION => Ok(arms.class(params, HookKind::BlockCodeExecution)),
        wire::COMPRESR => Ok(arms.class(params, HookKind::Compresr)),
        wire::CUSTOM_CODE => Ok(arms.class(params, HookKind::CustomCode)),
        wire::HIDDENLAYER => Ok(arms.class(params, HookKind::Hiddenlayer)),
        wire::JAVELIN => Ok(arms.class(params, HookKind::Javelin)),
        wire::LLM_AS_A_JUDGE => Ok(arms.class(params, HookKind::LlmAsAJudge)),
        wire::NOMA => Ok(arms.class(params, HookKind::Noma)),
        wire::NOMA_V2 => Ok(arms.class(params, HookKind::NomaV2)),
        wire::ONYX => Ok(arms.class(params, HookKind::Onyx)),
        wire::OVALIX => Ok(arms.class(params, HookKind::Ovalix)),
        wire::PANGEA => Ok(arms.class(params, HookKind::Pangea)),
        wire::REPELLOAI => Ok(arms.class(params, HookKind::Repelloai)),
        _other => return None,
    })
}
