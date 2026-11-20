use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;

// These folders' initializers build a class from ports only B8666's dispatch has
fn kind_maker(kind: &str) -> Option<fn(LitellmParams) -> HookKind> {
    Some(match kind {
        wire::BLOCK_CODE_EXECUTION => |params| HookKind::BlockCodeExecution(Box::new(params)),
        wire::COMPRESR => |params| HookKind::Compresr(Box::new(params)),
        wire::CUSTOM_CODE => |params| HookKind::CustomCode(Box::new(params)),
        wire::HIDDENLAYER => |params| HookKind::Hiddenlayer(Box::new(params)),
        wire::JAVELIN => |params| HookKind::Javelin(Box::new(params)),
        wire::LLM_AS_A_JUDGE => |params| HookKind::LlmAsAJudge(Box::new(params)),
        wire::NOMA => |params| HookKind::Noma(Box::new(params)),
        wire::NOMA_V2 => |params| HookKind::NomaV2(Box::new(params)),
        wire::ONYX => |params| HookKind::Onyx(Box::new(params)),
        wire::OVALIX => |params| HookKind::Ovalix(Box::new(params)),
        wire::PANGEA => |params| HookKind::Pangea(Box::new(params)),
        wire::REPELLOAI => |params| HookKind::Repelloai(Box::new(params)),
        _other => return None,
    })
}

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    _name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let Some(make) = kind_maker(kind) else {
        return None;
    };
    Some(Ok(HookSpec::new(settings, make(params.clone()))))
}
