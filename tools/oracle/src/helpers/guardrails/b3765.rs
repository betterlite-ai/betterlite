use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::compresr_writeback::knobs::Tuning;
use betterlite_config::sections::compresr_writeback::{Configured, UnreachableFallback};
use betterlite_guardrails::compresr_settings::error::CompresrSettingsError;
use betterlite_guardrails::compresr_settings::{CompresrGuardrail, CompresrRuntime, Compressible};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod apply;

#[derive(Deserialize)]
pub(crate) struct Config {
    api_base: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    #[serde(flatten)]
    tuning: Tuning,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Init {
        config: Config,
        env: BTreeMap<String, String>,
    },
    Apply(apply::Apply),
}

fn build(
    config: &Config,
    api_base: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<CompresrGuardrail, CompresrSettingsError> {
    let configured = Configured {
        guardrail_name: "squeeze".to_owned(),
        api_base: api_base
            .map(str::to_owned)
            .or_else(|| config.api_base.clone()),
        api_key: config.api_key.clone().map(SecretString::from),
        model: config.model.clone(),
        unreachable_fallback: UnreachableFallback::FailClosed,
        timeout: None,
        tuning: config.tuning.clone(),
    };
    let runtime = CompresrRuntime {
        clock: Box::new(|| Duration::ZERO),
        event_hook: None,
    };
    CompresrGuardrail::new(&configured, env, runtime, reqwest::Client::default())
}

fn settings_of(guardrail: &CompresrGuardrail) -> Value {
    let settings = &guardrail.settings;
    let transport = &guardrail.transport.settings;
    json!({
        "accepted": true,
        "api_base": transport.api_base,
        "model": transport.model,
        "ratio": transport.target_compression_ratio,
        "coarse": transport.coarse,
        "dynamic": transport.dynamic,
        "min_chars": settings.min_chars_to_compress,
        "tool_outputs": settings.targets.contains(&Compressible::ToolOutputs),
        "retrieval": guardrail.recovery.is_some(),
        "max_bytes": settings.max_bytes_per_call,
        "bypass": settings.allow_bypass_header,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Init { config, env } => Ok(match build(config, None, env) {
                Ok(guardrail) => settings_of(&guardrail),
                Err(_rejected) => json!({"accepted": false}),
            }),
            Input::Apply(call) => apply::run(call),
        }
    }
}
