#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::guardrail_translation::TranslationCall;
use betterlite_guardrails::unified_dispatch::registry::TranslationRegistry;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::bedrock_passthrough::text_paths::SkipFlags;
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::response::ChatResponse;
use crate::guardrails::responses_translation::block_events::BlockStamp;
use crate::guardrails::unified_chat::GuardrailToApply;
use crate::guardrails::unified_chat::stamps::StampSource;
use crate::guardrails::unified_guardrail::apply_record::{InvocationClock, RecordedGuardrail};
use crate::guardrails::unified_guardrail::entries::{RegistryConfig, build_registry};
use crate::guardrails::unified_guardrail::hooks::{
    GuardrailSkipDefaults, GuardrailToRun, UnifiedGuardrailHooks,
};
use crate::guardrails::unified_guardrail::recorder::AppliedGuardrailRecorder;
use crate::guardrails::unified_guardrail::translation::UnifiedTranslation;

pub(crate) const NAME: &str = "word-filter";

pub(crate) type Registry<G> =
    TranslationRegistry<UnifiedTranslation<G, FixedStamp, NoAssembler, NoHook>>;

pub(crate) type Hooks =
    UnifiedGuardrailHooks<FakeGuardrail, FixedStamp, NoAssembler, NoHook, NameLog>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SeenCall {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
}

// masks every text it is sent, or fails with the failure it holds
pub(crate) struct FakeGuardrail {
    pub(crate) failure: Option<GuardrailFailure>,
    pub(crate) calls: Mutex<Vec<SeenCall>>,
}

impl FakeGuardrail {
    pub(crate) fn masking() -> Arc<Self> {
        Arc::new(Self {
            failure: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Arc<Self> {
        Arc::new(Self {
            failure: Some(failure),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn seen(&self) -> Vec<SeenCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn seen_texts(&self) -> Vec<Vec<String>> {
        self.seen()
            .into_iter()
            .map(|call| call.inputs.texts.unwrap_or_default())
            .collect()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(SeenCall {
            inputs: inputs.clone(),
            input_type,
        });
        let mut masked = inputs;
        masked.texts = masked
            .texts
            .map(|texts| texts.iter().map(|_text| String::from("[masked]")).collect());
        ready(self.failure.clone().map_or(Ok(masked), Err))
    }
}

#[derive(Clone)]
pub(crate) struct FixedStamp;

impl StampSource for FixedStamp {
    fn stamp(&self) -> BlockStamp {
        BlockStamp {
            response_id: String::from("resp_fixed"),
            message_id: String::from("msg_fixed"),
            created_at: 1234_i64,
        }
    }
}

#[derive(Clone)]
pub(crate) struct NoAssembler;

impl ChunkAssembler for NoAssembler {
    fn assemble(
        &self,
        _chunks: &[Object],
    ) -> impl Future<Output = Result<ChatResponse, RewriteError>> + Send {
        ready(Err(RewriteError::NoStreamChoices))
    }
}

#[derive(Clone)]
pub(crate) struct NoHook;

impl PostCallSuccessHook for NoHook {
    type Error = Infallible;

    fn post_call_success_hook(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        _user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send {
        ready(Ok(response))
    }
}

#[derive(Default)]
pub(crate) struct SpanLog(pub(crate) Mutex<Vec<GuardrailInvocationRecord>>);

impl GuardrailSpanSink for SpanLog {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct NameLog(pub(crate) Mutex<Vec<Option<String>>>);

impl AppliedGuardrailRecorder for NameLog {
    fn record(
        &self,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        guardrail_name: Option<&str>,
    ) {
        self.0
            .lock()
            .unwrap()
            .push(guardrail_name.map(str::to_owned));
    }
}

pub(crate) struct FixedClock;

impl InvocationClock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .unwrap()
    }
}

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn object(json: &str) -> BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = value(json) else {
        panic!("expected a JSON object");
    };
    fields
}

pub(crate) fn no_call<'call>() -> TranslationCall<'call, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict: None,
        request_data: None,
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    }
}

pub(crate) fn blocked() -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: String::from("blocked"),
    }
}

pub(crate) fn principal() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"sub","auth_method":"api_key"}"#)
        .unwrap()
}

fn cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}

pub(crate) fn registry<G>() -> Registry<G>
where
    G: GuardrailContract + Send + Sync,
{
    build_registry(&RegistryConfig {
        stamps: FixedStamp,
        assembler: NoAssembler,
        limits: ApiLimits::default(),
        cache: Arc::new(cache()),
        hook: NoHook,
        bedrock_skip: SkipFlags {
            system: false,
            tool: false,
        },
        guardrail_name: None,
        passthrough_guardrails_config: None,
    })
}

pub(crate) fn applied(guardrail: &Arc<FakeGuardrail>) -> GuardrailToApply<FakeGuardrail> {
    GuardrailToApply {
        guardrail: Arc::clone(guardrail),
        guardrail_name: Some(NAME.to_owned()),
        principal: principal(),
        skip_flags: GuardrailSkipFlags::default(),
        global_flags: GlobalSkipFlags::default(),
    }
}

pub(crate) fn settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(NAME.to_owned()),
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches::default(),
    }
}

pub(crate) fn to_run(
    guardrail: &Arc<FakeGuardrail>,
    settings: GuardrailSettings,
) -> GuardrailToRun<FakeGuardrail> {
    GuardrailToRun {
        guardrail: Arc::clone(guardrail),
        base: Arc::new(GuardrailBase {
            settings,
            mode: None,
        }),
        skip_flags: GuardrailSkipFlags::default(),
    }
}

pub(crate) fn hooks(spans: &Arc<SpanLog>, skip_defaults: GuardrailSkipDefaults) -> Hooks {
    let sink: Arc<dyn GuardrailSpanSink + Send + Sync> = Arc::<SpanLog>::clone(spans);
    UnifiedGuardrailHooks {
        registry: registry::<RecordedGuardrail<FakeGuardrail>>(),
        recorder: NameLog::default(),
        routes: Arc::new(RouteCatalog::parse(CATALOG).unwrap()),
        sink,
        clock: Arc::new(FixedClock),
        skip_defaults,
    }
}
