use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::compresr_writeback::{
    Configured, UnreachableFallback as ConfiguredFallback,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use error::CompresrSettingsError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::compresr_recovery::store::RecoveryStore;
use crate::compresr_transport::{CompresrTransport, DEFAULT_COMPRESS_TIMEOUT, TransportSettings};
use crate::generic_guardrail_contracts::UnreachableFallback;

pub mod error;
mod tests;

pub const BYPASS_HEADER: &str = "x-compresr-bypass";
pub const DEFAULT_API_BASE: &str = "https://api.compresr.ai";
pub const DEFAULT_COMPRESSION_MODEL: &str = "latte_v2";
pub const DEFAULT_TARGET_COMPRESSION_RATIO: f64 = 0.5;
pub const DEFAULT_MIN_CHARS_TO_COMPRESS: usize = 500;
pub const DEFAULT_MAX_BYTES_PER_CALL: usize = 10 * 1024 * 1024;
pub const UI_FRIENDLY_NAME: &str = "Compresr (context compression)";

const API_BASE_ENV: &str = "COMPRESR_API_BASE";
const API_KEY_ENV: &str = "COMPRESR_API_KEY";
const PROXY_SERVER_REQUEST_FIELD: &str = "proxy_server_request";
const HEADERS_FIELD: &str = "headers";

pub type Clock = Box<dyn Fn() -> Duration + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Compressible {
    ToolOutputs,
    System,
    History,
    LastUser,
}

#[derive(Debug, Clone)]
pub struct CompresrSettings {
    pub min_chars_to_compress: usize,
    pub targets: BTreeSet<Compressible>,
    pub max_bytes_per_call: usize,
    pub allow_bypass_header: bool,
}

pub struct CompresrRuntime {
    pub clock: Clock,
    pub event_hook: Option<LitellmParamsMode>,
}

pub struct CompresrGuardrail {
    pub guardrail_name: String,
    pub settings: CompresrSettings,
    pub transport: CompresrTransport,
    // Present exactly when enable_retrieval is on
    pub recovery: Option<RecoveryStore>,
    pub runtime: CompresrRuntime,
    pub no_scope_warning_until: Mutex<Duration>,
}

// LiteLLM's `a or b` skips an empty value as well as an unset one
fn api_key(
    configured: &Configured,
    lookup: &dyn EnvironmentLookup,
) -> Result<SecretString, CompresrSettingsError> {
    configured
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            lookup
                .lookup(API_KEY_ENV)
                .filter(|text| !text.is_empty())
                .map(SecretString::from)
        })
        .ok_or(CompresrSettingsError::MissingApiKey)
}

fn transport_settings(
    configured: &Configured,
    lookup: &dyn EnvironmentLookup,
) -> Result<TransportSettings, CompresrSettingsError> {
    let tuning = &configured.tuning;
    let api_base = configured
        .api_base
        .clone()
        .filter(|text| !text.is_empty())
        .or_else(|| lookup.lookup(API_BASE_ENV).filter(|text| !text.is_empty()))
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let timeout = match configured.timeout {
        None => DEFAULT_COMPRESS_TIMEOUT,
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|source| CompresrSettingsError::Timeout { seconds, source })?,
    };
    Ok(TransportSettings {
        api_base,
        model: configured
            .model
            .clone()
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| DEFAULT_COMPRESSION_MODEL.to_owned()),
        target_compression_ratio: tuning
            .target_compression_ratio
            .unwrap_or(DEFAULT_TARGET_COMPRESSION_RATIO),
        coarse: tuning.coarse.unwrap_or(true),
        dynamic: tuning.dynamic.unwrap_or(true),
        dynamic_min_ratio: tuning.dynamic_min_ratio,
        dynamic_max_ratio: tuning.dynamic_max_ratio,
        compression_params: tuning.compression_params.clone().unwrap_or_default(),
        timeout,
        fallback: match configured.unreachable_fallback {
            ConfiguredFallback::FailOpen => UnreachableFallback::FailOpen,
            ConfiguredFallback::FailClosed => UnreachableFallback::FailClosed,
        },
    })
}

impl CompresrGuardrail {
    pub fn new(
        configured: &Configured,
        lookup: &dyn EnvironmentLookup,
        runtime: CompresrRuntime,
        client: reqwest::Client,
    ) -> Result<Self, CompresrSettingsError> {
        let transport = CompresrTransport::new(
            transport_settings(configured, lookup)?,
            api_key(configured, lookup)?,
            client,
        )?;
        let settings = CompresrSettings::resolve(configured)?;
        let recovery = configured
            .tuning
            .enable_retrieval
            .unwrap_or(true)
            .then(RecoveryStore::default);
        if recovery.is_some() {
            tracing::warn!(
                "Compresr: enable_retrieval is on; the recovery store is per-process. For multi-worker deployments, set enable_retrieval=false or run with --workers 1."
            );
        }
        Ok(Self {
            guardrail_name: configured.guardrail_name.clone(),
            settings,
            transport,
            recovery,
            runtime,
            no_scope_warning_until: Mutex::new(Duration::ZERO),
        })
    }

    // LiteLLM's _should_bypass: the header is caller-settable, so it counts only when the operator allows it
    #[must_use]
    pub fn should_bypass(&self, request_data: &BTreeMap<String, GuardrailValue>) -> bool {
        if !self.settings.allow_bypass_header {
            return false;
        }
        let Some(GuardrailValue::Object(request)) = request_data.get(PROXY_SERVER_REQUEST_FIELD)
        else {
            return false;
        };
        let Some(GuardrailValue::Object(headers)) = request.get(HEADERS_FIELD) else {
            return false;
        };
        matches!(
            headers.get(BYPASS_HEADER),
            Some(GuardrailValue::Text(value)) if value.eq_ignore_ascii_case("true")
        )
    }
}

impl CompresrSettings {
    fn resolve(configured: &Configured) -> Result<Self, CompresrSettingsError> {
        let tuning = &configured.tuning;
        let max_bytes_per_call = match tuning.max_bytes_per_call {
            None => DEFAULT_MAX_BYTES_PER_CALL,
            Some(cap) => {
                usize::try_from(cap).map_err(|source| CompresrSettingsError::ByteCap { source })?
            }
        };
        let mut targets = BTreeSet::new();
        for (target, enabled) in [
            (
                Compressible::ToolOutputs,
                tuning.compress_tool_outputs.unwrap_or(true),
            ),
            (
                Compressible::System,
                tuning.compress_system.unwrap_or(false),
            ),
            (
                Compressible::History,
                tuning.compress_history.unwrap_or(false),
            ),
            (
                Compressible::LastUser,
                tuning.compress_last_user.unwrap_or(false),
            ),
        ] {
            if enabled {
                targets.insert(target);
            }
        }
        // A negative minimum never skips a message, so it reads as no minimum
        let min_chars_to_compress = tuning
            .min_chars_to_compress
            .map_or(DEFAULT_MIN_CHARS_TO_COMPRESS, |minimum| {
                usize::try_from(minimum).unwrap_or_default()
            });
        Ok(Self {
            min_chars_to_compress,
            targets,
            max_bytes_per_call,
            allow_bypass_header: tuning.allow_bypass_header.unwrap_or(false),
        })
    }
}
