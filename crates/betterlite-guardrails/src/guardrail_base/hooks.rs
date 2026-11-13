use std::slice;

use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::{Error as TextError, StrDeserializer};

use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, ModeTarget};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::error::GuardrailBaseError;
use super::settings::GuardrailSwitches;

const fn wire_name(hook: GuardrailEventHooks) -> &'static str {
    match hook {
        GuardrailEventHooks::PreCall => "pre_call",
        GuardrailEventHooks::PostCall => "post_call",
        GuardrailEventHooks::DuringCall => "during_call",
        GuardrailEventHooks::LoggingOnly => "logging_only",
        GuardrailEventHooks::PreMcpCall => "pre_mcp_call",
        GuardrailEventHooks::DuringMcpCall => "during_mcp_call",
        GuardrailEventHooks::PostMcpCall => "post_mcp_call",
        GuardrailEventHooks::RealtimeInputTranscription => "realtime_input_transcription",
    }
}

fn target_names(target: &ModeTarget) -> &[String] {
    match target {
        ModeTarget::One(name) => slice::from_ref(name),
        ModeTarget::Many(names) => names,
    }
}

pub fn validate_event_hook(
    requested: Option<&LitellmParamsMode>,
    supported: &[GuardrailEventHooks],
    switches: &GuardrailSwitches,
) -> Result<(), GuardrailBaseError> {
    let Some(mode_hook) = requested else {
        return Ok(());
    };
    let names: Vec<&String> = match mode_hook {
        LitellmParamsMode::One(name) => vec![name],
        LitellmParamsMode::Many(names) => names.iter().collect(),
        LitellmParamsMode::Tagged(mode) => {
            // LiteLLM tests the default for truthiness, so an empty one is skipped
            let empty_default = ModeTarget::One(String::new());
            let default = mode
                .default
                .iter()
                .filter(|target| **target != empty_default);
            mode.tags
                .values()
                .chain(default)
                .flat_map(target_names)
                .collect()
        }
    };
    let logging_only_allowed =
        switches.uses_apply_guardrail_interface && !switches.use_native_lifecycle_hooks;
    for name in names {
        let text: StrDeserializer<'_, TextError> = name.as_str().into_deserializer();
        let hook = GuardrailEventHooks::deserialize(text)
            .map_err(|_unknown| GuardrailBaseError::UnknownEventHook { hook: name.clone() })?;
        let allowed = supported.contains(&hook)
            || (hook == GuardrailEventHooks::LoggingOnly && logging_only_allowed);
        if !allowed {
            let listed: Vec<&str> = supported
                .iter()
                .map(|candidate| wire_name(*candidate))
                .collect();
            return Err(GuardrailBaseError::UnsupportedEventHook {
                hook: name.clone(),
                supported: listed.join(", "),
            });
        }
    }
    Ok(())
}
