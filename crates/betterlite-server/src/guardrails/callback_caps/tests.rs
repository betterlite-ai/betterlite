#![cfg(test)]

use std::collections::BTreeMap;
use std::slice;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;

use super::caps::{CallbackCache, CallbackCapabilities, IteratorKind, combined_callback_list};
use super::errors::{
    GuardrailGateError, ModifiedResponseError, ProxyException, ProxyHookError, SensitiveRouteError,
    changes_request_flow, is_client_error,
};
use super::traits::{
    FailureReply, GuardrailCallback, GuardrailEvent, HeaderHooks, HeadersContext, HookOverrides,
    PostCallHooks, PreCallReturn, ProxyCallback, RequestHooks, StreamingHooks,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hook {
    PreCall,
    Moderation,
    PostCallSuccess,
    PostCallFailure,
    ResponseHeaders,
    StreamingChunk,
    StreamingIterator,
    ApplyGuardrail,
    FilterListedModels,
    AcceptsCallInfo,
}

fn overrides(hooks: &[Hook]) -> HookOverrides {
    HookOverrides {
        apply_guardrail: hooks.contains(&Hook::ApplyGuardrail),
        filter_listed_models: hooks.contains(&Hook::FilterListedModels),
        request: RequestHooks {
            pre_call: hooks.contains(&Hook::PreCall),
            moderation: hooks.contains(&Hook::Moderation),
        },
        post_call: PostCallHooks {
            success: hooks.contains(&Hook::PostCallSuccess),
            failure: hooks.contains(&Hook::PostCallFailure),
        },
        headers: HeaderHooks {
            response_headers: hooks.contains(&Hook::ResponseHeaders),
            accepts_call_info: hooks.contains(&Hook::AcceptsCallInfo),
        },
        streaming: StreamingHooks {
            chunk: hooks.contains(&Hook::StreamingChunk),
            iterator: hooks.contains(&Hook::StreamingIterator),
        },
    }
}

struct FakeGuardrail;

impl GuardrailCallback for FakeGuardrail {
    fn guardrail_name(&self) -> Option<&str> {
        Some("fake")
    }

    fn run_in_parallel(&self) -> bool {
        false
    }

    fn scan_raw_request(&self) -> bool {
        false
    }

    fn use_native_during_call_hook(&self) -> bool {
        false
    }

    fn event_hook_is_set(&self) -> bool {
        false
    }

    fn moderation_check(&self) -> Option<&str> {
        None
    }

    fn should_run_guardrail(
        &self,
        _data: &JsonMap,
        _event: GuardrailEvent,
    ) -> Result<bool, GuardrailGateError> {
        Ok(true)
    }

    fn mark_pre_call_hook_ran(&self, _data: &mut JsonMap) {}
}

#[derive(Debug, Clone, Copy, Default)]
struct FakeFlags {
    base_logger: bool,
    content_enforcer: bool,
}

struct FakeCallback {
    name: &'static str,
    overrides: HookOverrides,
    flags: FakeFlags,
    native_lifecycle: bool,
    guardrail: Option<FakeGuardrail>,
}

impl ProxyCallback for FakeCallback {
    fn overrides(&self) -> HookOverrides {
        self.overrides
    }

    fn class_name(&self) -> &str {
        self.name
    }

    fn is_base_logger(&self) -> bool {
        self.flags.base_logger
    }

    fn enforces_request_content(&self) -> bool {
        self.flags.content_enforcer
    }

    fn use_native_lifecycle_hooks(&self) -> bool {
        self.native_lifecycle
    }

    fn guardrail(&self) -> Option<&dyn GuardrailCallback> {
        self.guardrail
            .as_ref()
            .map(|guardrail| -> &dyn GuardrailCallback { guardrail })
    }
}

fn fake(name: &'static str, hooks: &[Hook], guardrail: Option<FakeGuardrail>) -> FakeCallback {
    FakeCallback {
        name,
        overrides: overrides(hooks),
        flags: FakeFlags::default(),
        native_lifecycle: false,
        guardrail,
    }
}

fn callback(name: &'static str, hooks: &[Hook]) -> Arc<dyn ProxyCallback> {
    Arc::new(fake(name, hooks, None))
}

fn guardrail(name: &'static str, hooks: &[Hook]) -> Arc<dyn ProxyCallback> {
    Arc::new(fake(name, hooks, Some(FakeGuardrail)))
}

fn base_logger() -> Arc<dyn ProxyCallback> {
    let every_hook = [
        Hook::PreCall,
        Hook::Moderation,
        Hook::PostCallSuccess,
        Hook::PostCallFailure,
        Hook::ResponseHeaders,
        Hook::StreamingChunk,
        Hook::StreamingIterator,
        Hook::ApplyGuardrail,
        Hook::FilterListedModels,
        Hook::AcceptsCallInfo,
    ];
    Arc::new(FakeCallback {
        name: "CustomLogger",
        overrides: overrides(&every_hook),
        flags: FakeFlags {
            base_logger: true,
            content_enforcer: true,
        },
        native_lifecycle: false,
        guardrail: Some(FakeGuardrail),
    })
}

fn proxy_error(code: &str) -> ProxyHookError {
    ProxyHookError::Proxy(Box::new(ProxyException {
        message: "the proxy refused".to_owned(),
        error_type: "proxy_error".to_owned(),
        param: None,
        code: code.to_owned(),
        headers: BTreeMap::new(),
    }))
}

fn sensitive_route() -> ProxyHookError {
    ProxyHookError::SensitiveDataRoute(Box::new(SensitiveRouteError {
        session_id: "session-1".to_owned(),
        route_to_model: "on-prem".to_owned(),
        guardrail_name: Some("pii".to_owned()),
        sticky_session_routing: true,
        detection_info: BTreeMap::new(),
    }))
}

fn modified_response() -> ProxyHookError {
    ProxyHookError::ModifyResponse(Box::new(ModifiedResponseError {
        message: "content blocked".to_owned(),
        model: "gpt-4o".to_owned(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response: None,
    }))
}

fn guardrail_raised() -> ProxyHookError {
    ProxyHookError::GuardrailRaised {
        message: "guardrail raised an exception".to_owned(),
        guardrail_name: None,
        status: 400,
        blocked_content: true,
    }
}

fn rejected_request() -> ProxyHookError {
    ProxyHookError::RejectedRequest {
        message: "not allowed".to_owned(),
        model: "gpt-4o".to_owned(),
    }
}

#[test]
fn from_callbacks_defaults_to_no_flags() {
    let capabilities = CallbackCapabilities::from_callbacks(&[]);
    assert!(!capabilities.has_post_call_response_headers);
    assert!(!capabilities.streaming.has_iterator_override);
    assert!(!capabilities.streaming.has_streaming_chunk_override);
    assert!(!capabilities.guardrails.has_guardrail);
    assert!(!capabilities.request.has_pre_call_override);
    assert!(!capabilities.request.has_content_enforcer);
    assert!(!capabilities.guardrails.has_moderation_override);
    assert!(capabilities.iterator_overrides.is_empty());
    assert!(capabilities.resolved_callbacks.is_empty());
    assert!(capabilities.listed_models_filters.is_empty());
}

#[test]
fn from_callbacks_counts_response_headers_and_chunk_overrides() {
    let capabilities = CallbackCapabilities::from_callbacks(&[
        callback("headers", &[Hook::ResponseHeaders, Hook::AcceptsCallInfo]),
        callback("chunk", &[Hook::StreamingChunk]),
    ]);
    assert!(capabilities.has_post_call_response_headers);
    assert!(capabilities.streaming.has_streaming_chunk_override);
    assert!(!capabilities.streaming.has_iterator_override);
}

#[test]
fn overrides_carry_the_call_info_flag() {
    let declaring = callback("headers", &[Hook::ResponseHeaders, Hook::AcceptsCallInfo]);
    let ignoring = callback("headers", &[Hook::ResponseHeaders]);
    assert!(declaring.overrides().headers.accepts_call_info);
    assert!(!ignoring.overrides().headers.accepts_call_info);
}

#[test]
fn from_callbacks_keeps_guardrails_out_of_moderation_override() {
    let with_guardrail =
        CallbackCapabilities::from_callbacks(&[guardrail("pii", &[Hook::Moderation])]);
    assert!(with_guardrail.guardrails.has_guardrail);
    assert!(!with_guardrail.guardrails.has_moderation_override);

    let with_logger =
        CallbackCapabilities::from_callbacks(&[callback("logger", &[Hook::Moderation])]);
    assert!(!with_logger.guardrails.has_guardrail);
    assert!(with_logger.guardrails.has_moderation_override);
}

#[test]
fn from_callbacks_records_one_override_for_a_type_with_both_hooks() {
    let both = callback("both", &[Hook::StreamingIterator, Hook::ApplyGuardrail]);
    let capabilities = CallbackCapabilities::from_callbacks(&[Arc::clone(&both)]);
    assert_eq!(capabilities.iterator_overrides.len(), 1);
    let entry = capabilities
        .iterator_overrides
        .first()
        .expect("the callback defines the iterator hook");
    assert_eq!(entry.kind, IteratorKind::Override);
    assert!(Arc::ptr_eq(&entry.callback, &both));
    assert!(capabilities.streaming.has_iterator_override);
}

#[test]
fn from_callbacks_wraps_apply_guardrail_only_without_native_hooks() {
    let unified = callback("unified", &[Hook::ApplyGuardrail]);
    let native: Arc<dyn ProxyCallback> = Arc::new(FakeCallback {
        name: "native",
        overrides: overrides(&[Hook::ApplyGuardrail]),
        flags: FakeFlags::default(),
        native_lifecycle: true,
        guardrail: Some(FakeGuardrail),
    });
    let capabilities =
        CallbackCapabilities::from_callbacks(&[Arc::clone(&unified), Arc::clone(&native)]);
    assert_eq!(capabilities.iterator_overrides.len(), 1);
    let entry = capabilities
        .iterator_overrides
        .first()
        .expect("the unified callback gets an entry");
    assert_eq!(entry.kind, IteratorKind::ApplyGuardrail);
    assert!(Arc::ptr_eq(&entry.callback, &unified));
    assert!(capabilities.streaming.has_iterator_override);
}

#[test]
fn from_callbacks_needs_a_pre_call_override_for_the_content_enforcer() {
    let enforcer: Arc<dyn ProxyCallback> = Arc::new(FakeCallback {
        name: "enforcer",
        overrides: overrides(&[Hook::PreCall]),
        flags: FakeFlags {
            base_logger: false,
            content_enforcer: true,
        },
        native_lifecycle: false,
        guardrail: None,
    });
    let enforcing = CallbackCapabilities::from_callbacks(&[enforcer]);
    assert!(enforcing.request.has_pre_call_override);
    assert!(enforcing.request.has_content_enforcer);

    let plain = CallbackCapabilities::from_callbacks(&[callback("plain", &[Hook::PreCall])]);
    assert!(plain.request.has_pre_call_override);
    assert!(!plain.request.has_content_enforcer);

    let bare: Arc<dyn ProxyCallback> = Arc::new(FakeCallback {
        name: "enforcer",
        overrides: overrides(&[]),
        flags: FakeFlags {
            base_logger: false,
            content_enforcer: true,
        },
        native_lifecycle: false,
        guardrail: None,
    });
    let without_hook = CallbackCapabilities::from_callbacks(&[bare]);
    assert!(!without_hook.request.has_pre_call_override);
    assert!(!without_hook.request.has_content_enforcer);
}

#[test]
fn from_callbacks_keeps_the_list_order() {
    let first = callback("first", &[Hook::FilterListedModels]);
    let second = guardrail("second", &[Hook::StreamingIterator]);
    let third = callback("third", &[Hook::ApplyGuardrail]);
    let fourth = callback("fourth", &[Hook::FilterListedModels]);
    let capabilities = CallbackCapabilities::from_callbacks(&[first, second, third, fourth]);

    let resolved: Vec<&str> = capabilities
        .resolved_callbacks
        .iter()
        .map(|callback| callback.class_name())
        .collect();
    assert_eq!(resolved, ["first", "second", "third", "fourth"]);

    let iterator: Vec<(&str, IteratorKind)> = capabilities
        .iterator_overrides
        .iter()
        .map(|entry| (entry.callback.class_name(), entry.kind))
        .collect();
    assert_eq!(
        iterator,
        [
            ("second", IteratorKind::Override),
            ("third", IteratorKind::ApplyGuardrail),
        ]
    );

    let filters: Vec<&str> = capabilities
        .listed_models_filters
        .iter()
        .map(|callback| callback.class_name())
        .collect();
    assert_eq!(filters, ["first", "fourth"]);
}

#[test]
fn from_callbacks_skips_every_flag_of_the_base_logger() {
    let base = base_logger();
    let capabilities = CallbackCapabilities::from_callbacks(&[Arc::clone(&base)]);
    assert_eq!(capabilities.resolved_callbacks.len(), 1);
    let resolved = capabilities
        .resolved_callbacks
        .first()
        .expect("the base logger is still resolved");
    assert!(Arc::ptr_eq(resolved, &base));
    assert!(!capabilities.has_post_call_response_headers);
    assert!(!capabilities.streaming.has_streaming_chunk_override);
    assert!(!capabilities.guardrails.has_guardrail);
    assert!(!capabilities.request.has_pre_call_override);
    assert!(!capabilities.request.has_content_enforcer);
    assert!(!capabilities.guardrails.has_moderation_override);
    assert!(!capabilities.streaming.has_iterator_override);
    assert!(capabilities.iterator_overrides.is_empty());
    assert!(capabilities.listed_models_filters.is_empty());
}

#[test]
fn cache_reuses_the_capabilities_for_the_same_list() {
    let cache = CallbackCache::default();
    let callbacks = vec![callback("one", &[Hook::StreamingChunk])];
    let first = cache.capabilities(&callbacks);
    let second = cache.capabilities(&callbacks);
    assert!(Arc::ptr_eq(&first, &second));
    assert!(second.streaming.has_streaming_chunk_override);
}

#[test]
fn cache_recomputes_when_a_member_changes() {
    let cache = CallbackCache::default();
    let first = cache.capabilities(&[callback("one", &[Hook::StreamingChunk])]);
    let second = cache.capabilities(&[callback("one", &[Hook::StreamingChunk])]);
    assert!(!Arc::ptr_eq(&first, &second));
    let replaced = cache.capabilities(&[callback("one", &[])]);
    assert!(!Arc::ptr_eq(&second, &replaced));
    assert!(!replaced.streaming.has_streaming_chunk_override);
}

#[test]
fn cache_recomputes_when_the_length_changes() {
    let cache = CallbackCache::default();
    let one = callback("one", &[]);
    let two = callback("two", &[Hook::StreamingChunk]);
    let single = cache.capabilities(slice::from_ref(&one));
    let pair = cache.capabilities(&[Arc::clone(&one), Arc::clone(&two)]);
    assert!(!Arc::ptr_eq(&single, &pair));
    assert!(!single.streaming.has_streaming_chunk_override);
    assert!(pair.streaming.has_streaming_chunk_override);
    let back = cache.capabilities(slice::from_ref(&one));
    assert!(!Arc::ptr_eq(&back, &pair));
    assert!(!back.streaming.has_streaming_chunk_override);
}

#[test]
fn has_streaming_callbacks_covers_every_streaming_hook() {
    let empty = CallbackCapabilities::from_callbacks(&[]);
    let iterator =
        CallbackCapabilities::from_callbacks(&[callback("iterator", &[Hook::StreamingIterator])]);
    let chunk = CallbackCapabilities::from_callbacks(&[callback("chunk", &[Hook::StreamingChunk])]);
    let guarded = CallbackCapabilities::from_callbacks(&[guardrail("guard", &[])]);

    assert!(!empty.has_streaming_callbacks());
    assert!(iterator.has_streaming_callbacks());
    assert!(chunk.has_streaming_callbacks());
    assert!(guarded.has_streaming_callbacks());
}

#[test]
fn needs_per_chunk_hook_needs_a_chunk_override_or_a_guardrail() {
    let empty = CallbackCapabilities::from_callbacks(&[]);
    let iterator =
        CallbackCapabilities::from_callbacks(&[callback("iterator", &[Hook::StreamingIterator])]);
    let chunk = CallbackCapabilities::from_callbacks(&[callback("chunk", &[Hook::StreamingChunk])]);
    let guarded = CallbackCapabilities::from_callbacks(&[guardrail("guard", &[])]);
    let both = CallbackCapabilities::from_callbacks(&[
        callback("iterator", &[Hook::StreamingIterator]),
        callback("chunk", &[Hook::StreamingChunk]),
        guardrail("guard", &[]),
    ]);

    assert!(!empty.needs_per_chunk_hook);
    assert!(!iterator.needs_per_chunk_hook);
    assert!(chunk.needs_per_chunk_hook);
    assert!(guarded.needs_per_chunk_hook);
    assert!(both.needs_per_chunk_hook);
}

#[test]
fn partition_post_call_keeps_guardrails_first_in_order() {
    let logger_one = callback("logger-one", &[Hook::ResponseHeaders]);
    let guardrail_one = guardrail("guardrail-one", &[]);
    let logger_two = callback("logger-two", &[]);
    let guardrail_two = guardrail("guardrail-two", &[]);
    let capabilities = CallbackCapabilities::from_callbacks(&[
        logger_one,
        guardrail_one,
        logger_two,
        guardrail_two,
    ]);
    let (guardrails, others) = capabilities.partition_post_call();
    let guardrail_names: Vec<&str> = guardrails
        .iter()
        .map(|callback| callback.class_name())
        .collect();
    let other_names: Vec<&str> = others
        .iter()
        .map(|callback| callback.class_name())
        .collect();
    assert_eq!(guardrail_names, ["guardrail-one", "guardrail-two"]);
    assert_eq!(other_names, ["logger-one", "logger-two"]);
}

#[test]
fn combined_callback_list_returns_the_global_list_when_dynamic_is_none() {
    let first = callback("first", &[]);
    let second = callback("second", &[]);
    let global = [Arc::clone(&first), Arc::clone(&second), Arc::clone(&first)];
    let combined = combined_callback_list(None, &global);

    let names: Vec<&str> = combined
        .iter()
        .map(|callback| callback.class_name())
        .collect();
    assert_eq!(names, ["first", "second", "first"]);
    assert_eq!(combined.len(), 3);
    let head = combined.first().expect("the global list is kept");
    assert!(Arc::ptr_eq(head, &first));
}

#[test]
fn combined_callback_list_puts_dynamic_first_and_drops_later_duplicates() {
    let dynamic = callback("dynamic", &[]);
    let shared = callback("shared", &[]);
    let global = callback("global", &[]);
    let combined = combined_callback_list(
        Some(&[
            Arc::clone(&dynamic),
            Arc::clone(&shared),
            Arc::clone(&dynamic),
        ]),
        &[
            Arc::clone(&shared),
            Arc::clone(&global),
            Arc::clone(&global),
        ],
    );
    let names: Vec<&str> = combined
        .iter()
        .map(|callback| callback.class_name())
        .collect();
    assert_eq!(names, ["dynamic", "shared", "global"]);
    let kept = combined.get(1).expect("the dynamic copy is kept");
    assert!(Arc::ptr_eq(kept, &shared));
}

#[rstest]
#[case::http_bad_request(ProxyHookError::Http { status: 400, detail: "bad".to_owned() }, true)]
#[case::http_client_error(ProxyHookError::Http { status: 499, detail: "bad".to_owned() }, true)]
#[case::http_server_error(ProxyHookError::Http { status: 500, detail: "bad".to_owned() }, false)]
#[case::http_unavailable(ProxyHookError::Http { status: 503, detail: "bad".to_owned() }, false)]
#[case::proxy_rate_limit(proxy_error("429"), true)]
#[case::proxy_client_error(proxy_error("499"), true)]
#[case::proxy_server_error(proxy_error("500"), false)]
#[case::proxy_not_implemented(proxy_error("501"), false)]
#[case::proxy_padded_server_code(proxy_error("0500"), false)]
#[case::proxy_padded_client_code(proxy_error("000429"), true)]
#[case::proxy_text_code(proxy_error("not-a-code"), true)]
#[case::proxy_none_code(proxy_error("None"), true)]
#[case::proxy_empty_code(proxy_error(""), true)]
#[case::proxy_signed_code(proxy_error("+500"), true)]
#[case::proxy_spaced_code(proxy_error(" 500"), true)]
#[case::proxy_long_digit_code(proxy_error("999999999999999999999"), false)]
#[case::guardrail_raised(guardrail_raised(), false)]
#[case::sensitive_route(sensitive_route(), false)]
#[case::modified_response(modified_response(), false)]
#[case::rejected_request(rejected_request(), false)]
#[case::other(ProxyHookError::Other("boom".to_owned()), false)]
fn is_client_error_classifies_every_variant(#[case] error: ProxyHookError, #[case] expected: bool) {
    assert_eq!(is_client_error(&error), expected);
}

#[rstest]
#[case::sensitive_route(sensitive_route(), true)]
#[case::modified_response(modified_response(), true)]
#[case::http(ProxyHookError::Http { status: 400, detail: "bad".to_owned() }, false)]
#[case::proxy(proxy_error("429"), false)]
#[case::guardrail_raised(guardrail_raised(), false)]
#[case::rejected_request(rejected_request(), false)]
#[case::other(ProxyHookError::Other("boom".to_owned()), false)]
fn changes_request_flow_only_for_a_reroute_or_a_modified_response(
    #[case] error: ProxyHookError,
    #[case] expected: bool,
) {
    assert_eq!(changes_request_flow(&error), expected);
}

#[tokio::test]
async fn defaults_answer_not_overridden_for_request_hooks() {
    let plain = callback("plain", &[]);
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
        .expect("an empty auth dictionary builds a key");
    let mut data = JsonMap::new();
    let answer = plain
        .pre_call(&auth, &mut data, CallType::Acompletion)
        .await
        .expect("the default pre-call hook succeeds");
    assert_eq!(answer, PreCallReturn::Unchanged);
    plain
        .moderation(&mut data, Some(&auth), CallType::Acompletion)
        .await
        .expect("the default moderation hook succeeds");
}

#[tokio::test]
async fn defaults_answer_not_overridden_for_response_hooks() {
    let plain = callback("plain", &[]);
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
        .expect("an empty auth dictionary builds a key");
    let mut data = JsonMap::new();
    let replaced = plain
        .post_call_success(&mut data, Some(&auth), JsonValue::Null)
        .await
        .expect("the default success hook succeeds");
    assert_eq!(replaced, None);
    let transformed = plain
        .post_call_failure(&mut data, &guardrail_raised(), &auth, None)
        .await
        .expect("the default failure hook succeeds");
    assert_eq!(transformed, FailureReply::Original);
    let headers = plain
        .response_headers(&data, &auth, &JsonValue::Null, &HeadersContext::default())
        .await
        .expect("the default headers hook succeeds");
    assert!(headers.is_empty());
}

#[tokio::test]
async fn defaults_answer_not_overridden_for_streaming_and_listing() {
    let plain = callback("plain", &[]);
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
        .expect("an empty auth dictionary builds a key");
    let chunk = plain
        .streaming_chunk(&auth, "so far")
        .await
        .expect("the default chunk hook succeeds");
    assert_eq!(chunk, None);
    let names = vec!["gpt-4o".to_owned()];
    let kept = plain
        .filter_listed_models(&auth, names.clone())
        .await
        .expect("the default listing filter succeeds");
    assert_eq!(kept, names);
}
