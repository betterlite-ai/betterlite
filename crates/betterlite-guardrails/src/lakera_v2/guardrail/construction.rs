use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use secrecy::SecretString;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_initializers::lakera_v2::LakeraV2Args;
use crate::guardrail_message_scope::scoped_indices::GuardrailSkipFlags;

use super::super::advisory;
use super::super::error::LakeraV2Error;
use super::{DEFAULT_API_BASE, LakeraV2Flags, LakeraV2Guardrail};

impl LakeraV2Guardrail {
    pub fn new(
        args: LakeraV2Args,
        base: GuardrailBase,
        client: reqwest::Client,
        env: &dyn EnvironmentLookup,
    ) -> Result<Self, LakeraV2Error> {
        let api_key = resolve_api_key(&args, env);
        let api_base = resolve_api_base(&args, env);
        let timeout = resolve_timeout(args.timeout)?;
        let flags = LakeraV2Flags {
            payload: args.payload.unwrap_or(true),
            breakdown: args.breakdown.unwrap_or(true),
            dev_info: args.dev_info.unwrap_or(true),
        };
        let on_flagged = args.on_flagged.unwrap_or(LakeraOnFlagged::Block);
        advisory::validate_advisory_config(
            on_flagged,
            args.advisory_system_message.as_deref(),
            Some(flags.payload),
            Some(flags.breakdown),
        )?;
        let skip_flags = GuardrailSkipFlags {
            skip_system_message: args.skip_system_message,
            skip_tool_message: args.skip_tool_message_in_guardrail,
            scan_only_tool_results: None,
        };
        Ok(Self {
            base: named_base(base, &args.name),
            client,
            api_key,
            api_base,
            project_id: args.project_id,
            metadata: args.metadata,
            flags,
            on_flagged,
            advisory_system_message: args.advisory_system_message,
            skip_flags,
            timeout,
        })
    }
}

// LiteLLM keeps the CustomGuardrail name, which the initializer already put on the base
fn named_base(mut base: GuardrailBase, name: &str) -> GuardrailBase {
    if base.settings.guardrail_name.is_none() && !name.is_empty() {
        base.settings.guardrail_name = Some(name.to_owned());
    }
    base
}

fn resolve_api_key(args: &LakeraV2Args, env: &dyn EnvironmentLookup) -> SecretString {
    match &args.api_key {
        Some(key) => key.clone(),
        None => SecretString::from(env.lookup("LAKERA_API_KEY").unwrap_or_default()),
    }
}

fn resolve_api_base(args: &LakeraV2Args, env: &dyn EnvironmentLookup) -> String {
    if let Some(api_base) = &args.api_base {
        return api_base.clone();
    }
    match env.lookup("LAKERA_API_BASE") {
        Some(api_base) if !api_base.is_empty() => api_base,
        _ => DEFAULT_API_BASE.to_owned(),
    }
}

pub(super) fn resolve_timeout(timeout: Option<f64>) -> Result<Option<Duration>, LakeraV2Error> {
    let Some(seconds) = timeout else {
        return Ok(None);
    };
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|source| LakeraV2Error::Timeout { seconds, source })
}
