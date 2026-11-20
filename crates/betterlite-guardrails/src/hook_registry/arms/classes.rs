use betterlite_config::sections::guardrail_params::params::LitellmParams;

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
    Some(match kind {
        wire::BLOCK_CODE_EXECUTION => Ok(HookSpec::new(
            settings,
            HookKind::BlockCodeExecution(Box::new(params.clone())),
        )),
        wire::COMPRESR => Ok(HookSpec::new(
            settings,
            HookKind::Compresr(Box::new(params.clone())),
        )),
        wire::CUSTOM_CODE => Ok(HookSpec::new(
            settings,
            HookKind::CustomCode(Box::new(params.clone())),
        )),
        wire::HIDDENLAYER => Ok(HookSpec::new(
            settings,
            HookKind::Hiddenlayer(Box::new(params.clone())),
        )),
        wire::JAVELIN => Ok(HookSpec::new(
            settings,
            HookKind::Javelin(Box::new(params.clone())),
        )),
        wire::LLM_AS_A_JUDGE => Ok(HookSpec::new(
            settings,
            HookKind::LlmAsAJudge(Box::new(params.clone())),
        )),
        wire::NOMA => Ok(HookSpec::new(
            settings,
            HookKind::Noma(Box::new(params.clone())),
        )),
        wire::NOMA_V2 => Ok(HookSpec::new(
            settings,
            HookKind::NomaV2(Box::new(params.clone())),
        )),
        wire::ONYX => Ok(HookSpec::new(
            settings,
            HookKind::Onyx(Box::new(params.clone())),
        )),
        wire::OVALIX => Ok(HookSpec::new(
            settings,
            HookKind::Ovalix(Box::new(params.clone())),
        )),
        wire::PANGEA => Ok(HookSpec::new(
            settings,
            HookKind::Pangea(Box::new(params.clone())),
        )),
        wire::REPELLOAI => Ok(HookSpec::new(
            settings,
            HookKind::Repelloai(Box::new(params.clone())),
        )),
        _other => return None,
    })
}
