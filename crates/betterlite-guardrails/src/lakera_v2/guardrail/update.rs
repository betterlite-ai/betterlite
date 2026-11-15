use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use serde::de::DeserializeOwned;

use super::super::advisory;
use super::super::error::LakeraV2Error;
use super::LakeraV2Guardrail;
use super::construction::resolve_timeout;

pub fn update_params(
    guardrail: &mut LakeraV2Guardrail,
    params: &LitellmParams,
) -> Result<(), LakeraV2Error> {
    let extra = &params.base.extra;
    // an absent key or an explicit null takes the field default, where LiteLLM's setattr loop would store None
    let on_flagged = read_param(extra, "on_flagged", Some(LakeraOnFlagged::Block))?;
    let template = read_param::<String>(extra, "advisory_system_message", None)?;
    let payload = read_param(extra, "payload", Some(true))?;
    let breakdown = read_param(extra, "breakdown", Some(true))?;
    let dev_info = read_param(extra, "dev_info", Some(true))?;
    let timeout = resolve_timeout(params.base.fields.timeout)?;
    // a rejected update leaves the live instance untouched, so validate before any write
    advisory::validate_advisory_config(
        on_flagged.unwrap_or(LakeraOnFlagged::Block),
        template.as_deref(),
        payload,
        breakdown,
    )?;
    guardrail.base.update_params(params);
    guardrail.on_flagged = on_flagged.unwrap_or(LakeraOnFlagged::Block);
    guardrail.flags.payload = payload.unwrap_or(true);
    guardrail.flags.breakdown = breakdown.unwrap_or(true);
    guardrail.flags.dev_info = dev_info.unwrap_or(true);
    guardrail.advisory_system_message = template;
    guardrail.timeout = timeout;
    guardrail.skip_flags.skip_system_message = params.base.fields.skip_system_message;
    guardrail.skip_flags.skip_tool_message = params.base.fields.skip_tool_message_in_guardrail;
    guardrail.skip_flags.scan_only_tool_results = params.base.fields.scan_only_tool_results;
    // the base setattr writes mode onto self.mode, while dispatch reads the event hook
    guardrail.base.settings.event_hook = Some(params.mode.clone());
    Ok(())
}

fn read_param<T>(
    extra: &OpaqueParams,
    key: &str,
    default: Option<T>,
) -> Result<Option<T>, LakeraV2Error>
where
    T: DeserializeOwned,
{
    match extra.0.get(key) {
        None | Some(GuardrailValue::Null) => Ok(default),
        Some(value) => serde_json::to_value(value)
            .and_then(serde_json::from_value::<T>)
            .map(Some)
            .map_err(|source| LakeraV2Error::InvalidParam {
                field: key.to_owned(),
                source,
            }),
    }
}
