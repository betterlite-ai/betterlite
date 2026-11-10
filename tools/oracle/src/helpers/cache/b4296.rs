use std::sync::Arc;
use std::sync::{Mutex, PoisonError};

use betterlite_cache::cache_key::derive::{get_cache_key, should_use_cache};
use betterlite_cache::cache_key::request::{
    CacheKeyRequest, KeyParam, LiteLlmParams, RequestCacheControl,
};
use betterlite_cache::cache_key::scope::{
    END_USER_SCOPE_FIELD, SEMANTIC_EXCLUDED_PARAMS, TENANT_SCOPE_FIELDS,
};
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::KeyObject;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    GetCacheKey(Box<CacheKeyInput>),
    ShouldUseCache(ShouldUseCacheInput),
    PrintVerbose(PrintVerboseInput),
    CacheModeValues,
    SemanticScopeValues,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheKeyInput {
    cache_type: LiteLlmCacheType,
    namespace: Option<String>,
    semantic_cache_scope: SemanticCacheScope,
    #[serde(default)]
    include_provider_params: bool,
    #[serde(default)]
    params: Vec<KeyParam>,
    metadata: Option<KeyObject>,
    litellm_metadata: Option<KeyObject>,
    litellm_params: Option<LiteLlmParams>,
    cache_control: Option<RequestCacheControl>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShouldUseCacheInput {
    mode: CacheMode,
    cache_control: Option<RequestCacheControl>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrintVerboseInput {
    message: String,
    set_verbose: bool,
}

#[derive(Default)]
struct RecordingLogger {
    records: Mutex<Vec<Payload>>,
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct RecordingConsole {
    prints: Mutex<Vec<Payload>>,
}

impl ConsoleSink for RecordingConsole {
    fn print(&self, payload: &Payload) -> bool {
        self.prints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }
}

type Hooks = VerboseHooks<RecordingLogger, RecordingConsole>;

fn hooks(set_verbose: bool) -> Result<Hooks, Box<crate::helpers::HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole::default(),
        set_verbose: Payload::Bool(set_verbose),
        redactor: Arc::new(redactor),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::GetCacheKey(case) => cache_key(case),
            Input::ShouldUseCache(case) => Ok(json!({
                "use_cache": should_use_cache(
                    &CacheKeySettings {
                        cache_type: LiteLlmCacheType::Local,
                        mode: case.mode,
                        namespace: None,
                        semantic_cache_scope: SemanticCacheScope::Key,
                        include_provider_params: false,
                    },
                    case.cache_control.as_ref(),
                ),
            })),
            Input::PrintVerbose(case) => print_verbose(case),
            Input::CacheModeValues => Ok(json!({
                "modes": wire_values([CacheMode::DefaultOn, CacheMode::DefaultOff])?,
            })),
            Input::SemanticScopeValues => Ok(json!({
                "scopes": wire_values([SemanticCacheScope::Key, SemanticCacheScope::EndUser])?,
                "tenant_fields": TENANT_SCOPE_FIELDS,
                "end_user_field": END_USER_SCOPE_FIELD,
                "excluded_params": SEMANTIC_EXCLUDED_PARAMS,
            })),
        }
    }
}

fn cache_key(case: &CacheKeyInput) -> crate::helpers::HelperResult {
    let mut request = CacheKeyRequest {
        params: case.params.clone(),
        metadata: case.metadata.clone(),
        litellm_metadata: case.litellm_metadata.clone(),
        litellm_params: case.litellm_params.clone(),
        cache_control: case.cache_control.clone(),
    };
    let key_settings = CacheKeySettings {
        cache_type: case.cache_type,
        mode: CacheMode::DefaultOn,
        namespace: case.namespace.clone(),
        semantic_cache_scope: case.semantic_cache_scope,
        include_provider_params: case.include_provider_params,
    };
    let fixture_hooks = hooks(false)?;
    let key = get_cache_key(&mut request, &key_settings, &fixture_hooks).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let preset = request
        .litellm_params
        .and_then(|params| params.preset_cache_key);
    Ok(json!({"cache_key": key, "preset_cache_key": preset}))
}

fn print_verbose(case: &PrintVerboseInput) -> crate::helpers::HelperResult {
    let fixture_hooks = hooks(case.set_verbose)?;
    let _printed = fixture_hooks.main_print_verbose(Payload::String(case.message.clone()));
    let printed = fixture_hooks
        .console
        .prints
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|payload| match payload {
            Payload::String(text) => text.clone(),
            Payload::Null
            | Payload::Bool(_)
            | Payload::Number(_)
            | Payload::Array(_)
            | Payload::Object(_) => payload.to_string(),
        })
        .collect::<Vec<_>>();
    Ok(json!({"console": printed}))
}

fn wire_values<Value>(values: [Value; 2]) -> Result<[String; 2], Box<crate::helpers::HelperError>>
where
    Value: serde::Serialize,
{
    let mut wires = [String::new(), String::new()];
    for (slot, value) in wires.iter_mut().zip(values) {
        let wire = serde_json::to_value(value)?;
        *slot = match &wire {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_) => wire.to_string(),
        };
    }
    Ok(wires)
}

pub(crate) struct Op;
