use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::types::Harness;

use super::config::CODEX_TOKEN_ENV;

pub(super) fn provider_overrides<S>(
    context: &SessionContext<S>,
) -> Result<Vec<String>, HarnessError> {
    let gateway = context
        .settings
        .gateway
        .as_ref()
        .ok_or(HarnessError::OptionsMismatch {
            given: "SessionSettings.gateway",
            harness: Harness::Codex,
            expected: "a configured session model endpoint",
        })?;
    let base = format!("{}/v1", gateway.api_base.trim_end_matches('/'));
    Ok(vec![
        "model_provider=litellm".to_owned(),
        "model_providers.litellm.name=litellm".to_owned(),
        format!(
            "model_providers.litellm.base_url={}",
            serde_json::Value::String(base)
        ),
        format!("model_providers.litellm.env_key={CODEX_TOKEN_ENV}"),
        "model_providers.litellm.wire_api=responses".to_owned(),
        "approval_policy=never".to_owned(),
    ])
}
