use super::super::error::HookRegistryError;
use super::super::spec::HookSpec;
use super::super::wire;

// Their class tasks are deferred, so these names must not fall through to Unsupported
pub(super) fn build(kind: &str) -> Option<Result<HookSpec, HookRegistryError>> {
    match kind {
        wire::DECISION_MODEL
        | wire::PROMPT_SECURITY
        | wire::MCP_END_USER_PERMISSION
        | wire::MCP_SECURITY => Some(Err(HookRegistryError::NotBuilt {
            kind: kind.to_owned(),
        })),
        _other => None,
    }
}
