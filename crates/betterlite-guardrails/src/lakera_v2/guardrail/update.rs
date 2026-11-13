use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use serde::de::DeserializeOwned;

use super::super::advisory;
use super::super::error::LakeraV2Error;
use super::LakeraV2Guardrail;
use super::construction::resolve_timeout;

impl LakeraV2Guardrail {
    pub fn update_in_memory_litellm_params(
        &mut self,
        params: &LitellmParams,
    ) -> Result<(), LakeraV2Error> {
        let extra = &params.base.extra;
        let on_flagged = read_option::<LakeraOnFlagged>(extra, "on_flagged")?;
        let template = read_option::<String>(extra, "advisory_system_message")?;
        let payload = read_option::<bool>(extra, "payload")?;
        let breakdown = read_option::<bool>(extra, "breakdown")?;
        let dev_info = read_option::<bool>(extra, "dev_info")?;
        let timeout = resolve_timeout(params.base.fields.timeout)?;
        let prospective_on_flagged = on_flagged.unwrap_or(self.on_flagged);
        let prospective_payload = payload.unwrap_or(self.flags.payload);
        let prospective_breakdown = breakdown.unwrap_or(self.flags.breakdown);
        // a rejected update leaves the live instance untouched, so validate before any write
        advisory::validate_advisory_config(
            prospective_on_flagged,
            template.as_deref(),
            Some(prospective_payload),
            Some(prospective_breakdown),
        )?;
        self.base.update_params(params);
        self.on_flagged = prospective_on_flagged;
        self.flags.payload = prospective_payload;
        self.flags.breakdown = prospective_breakdown;
        self.flags.dev_info = dev_info.unwrap_or(self.flags.dev_info);
        self.advisory_system_message = template;
        self.timeout = timeout;
        self.skip_flags.skip_system_message = params.base.fields.skip_system_message;
        self.skip_flags.skip_tool_message = params.base.fields.skip_tool_message_in_guardrail;
        self.skip_flags.scan_only_tool_results = params.base.fields.scan_only_tool_results;
        // the base setattr writes mode onto self.mode, while dispatch reads the event hook
        self.base.settings.event_hook = Some(params.mode.clone());
        Ok(())
    }
}

fn read_option<T>(extra: &OpaqueParams, key: &str) -> Result<Option<T>, LakeraV2Error>
where
    T: DeserializeOwned,
{
    match extra.0.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => serde_json::to_value(value)
            .and_then(serde_json::from_value::<T>)
            .map(Some)
            .map_err(|source| LakeraV2Error::InvalidParam {
                field: key.to_owned(),
                source,
            }),
    }
}
