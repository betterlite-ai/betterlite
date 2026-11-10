#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use rstest::rstest;
use serde_json::Number;

use super::derive::{get_cache_key, should_use_cache};
use super::error::CacheKeyError;
use super::request::{CacheKeyRequest, KeyParam, LiteLlmParams, RequestCacheControl};
use super::settings::CacheKeySettings;
use super::value::{KeyObject, KeyValue};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::semantic::SemanticCacheScope;

const EXACT_BASIC: &str = "06e2f4225b0c6450ad4bcab265986caa028bc9d052873a941da3667c8144e735";
const SEMANTIC_BASIC: &str = "df26c35c0625f22acf8563f29da50736fe960904269f7001f8fc3f20c698cacc";
const MODEL_ONLY: &str = "62402ad4a1a8fad610a2506ec80432e56843b4c6141bd3808751273d2ee5c23b";
const API_KEY_SCOPE: &str = "a9a6cfa238c963b51175393587926bf0b391cfc4f51975bdd67a3b07af66941d";
const KEY_AND_TEMPERATURE: &str =
    "eda26307e635225bf1602d09e1a9f5174c830132dd1e3982eca6d6a8cc0f9c52";
const PROVIDER_BETA: &str = "e2c873ccdd73a022390b1c7833481cd27909eb511ef458e7d3f1980f3eeaf492";

#[derive(Default)]
struct RecordingLogger {
    records: Mutex<Vec<Payload>>,
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .expect("fixture log")
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
            .expect("fixture console")
            .push(payload.clone());
        true
    }
}

type Hooks = VerboseHooks<RecordingLogger, RecordingConsole>;

fn hooks(set_verbose: bool) -> Hooks {
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole::default(),
        set_verbose: Payload::Bool(set_verbose),
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles"),
        ),
    }
}

fn console(hooks: &Hooks) -> Vec<Payload> {
    hooks
        .console
        .prints
        .lock()
        .expect("fixture console")
        .clone()
}

fn logged(hooks: &Hooks) -> Vec<Payload> {
    hooks.logger.records.lock().expect("fixture log").clone()
}

fn string(text: &str) -> KeyValue {
    KeyValue::String(text.to_owned())
}

fn number(value: f64) -> KeyValue {
    KeyValue::Number(Number::from_f64(value).expect("a finite number"))
}

fn integer(value: i64) -> KeyValue {
    KeyValue::Number(Number::from(value))
}

fn object(pairs: &[(&str, KeyValue)]) -> KeyObject {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn param(name: &str, value: KeyValue) -> KeyParam {
    KeyParam {
        name: name.to_owned(),
        value,
    }
}

fn with_params(params: Vec<KeyParam>) -> CacheKeyRequest {
    CacheKeyRequest {
        params,
        metadata: None,
        litellm_metadata: None,
        litellm_params: None,
        cache_control: None,
    }
}

fn settings_for(cache_type: LiteLlmCacheType) -> CacheKeySettings {
    CacheKeySettings {
        cache_type,
        mode: CacheMode::DefaultOn,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

fn empty_litellm_params() -> LiteLlmParams {
    LiteLlmParams {
        preset_cache_key: None,
        metadata: None,
        litellm_metadata: None,
        file_name: None,
    }
}

fn derive(request: &mut CacheKeyRequest, settings: &CacheKeySettings) -> String {
    let hooks = hooks(false);
    get_cache_key(request, settings, &hooks).expect("a cache key")
}

fn stored_preset(request: &CacheKeyRequest) -> Option<String> {
    let params = request.litellm_params.as_ref()?;
    params.preset_cache_key.clone()
}

#[test]
fn exact_key_hashes_the_request_material() {
    let messages = KeyValue::Array(vec![KeyValue::Object(object(&[
        ("role", string("user")),
        ("content", string("hi")),
    ]))]);
    let mut request = with_params(vec![
        param("model", string("gpt-4")),
        param("messages", messages),
        param("temperature", number(0.7)),
        param("max_tokens", integer(100)),
    ]);
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(key, EXACT_BASIC);
}

#[test]
fn semantic_key_omits_messages_prompt_and_input() {
    let key_for = |content: &str| {
        let messages = KeyValue::Array(vec![KeyValue::Object(object(&[(
            "content",
            string(content),
        )]))]);
        let mut request = with_params(vec![
            param("model", string("gpt-4")),
            param("messages", messages),
            param("prompt", string("a prompt")),
            param("input", string("an input")),
            param("system", string("be nice")),
            param("temperature", number(0.7)),
        ]);
        derive(&mut request, &settings_for(LiteLlmCacheType::RedisSemantic))
    };
    assert_eq!(key_for("hi"), SEMANTIC_BASIC);
    assert_eq!(key_for("a different wording"), SEMANTIC_BASIC);
}

#[test]
fn exact_key_keeps_the_prompt_content() {
    let mut first = with_params(vec![
        param("model", string("gpt-4")),
        param(
            "messages",
            KeyValue::Array(vec![KeyValue::Object(object(&[("content", string("hi"))]))]),
        ),
    ]);
    let mut second = with_params(vec![
        param("model", string("gpt-4")),
        param(
            "messages",
            KeyValue::Array(vec![KeyValue::Object(object(&[(
                "content",
                string("bye"),
            )]))]),
        ),
    ]);
    let settings = settings_for(LiteLlmCacheType::Local);
    assert_ne!(
        derive(&mut first, &settings),
        derive(&mut second, &settings)
    );
}

#[test]
fn tenant_scope_appends_the_api_key() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[("user_api_key", string("key-hash"))]));
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::RedisSemantic));
    assert_eq!(key, API_KEY_SCOPE);
}

#[test]
fn tenant_scope_appends_team_and_org_ids() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[
        ("user_api_key", string("key-hash")),
        ("user_api_key_team_id", string("team-id")),
        ("user_api_key_org_id", string("org-id")),
    ]));
    let key = derive(
        &mut request,
        &settings_for(LiteLlmCacheType::ValkeySemantic),
    );
    assert_eq!(
        key,
        "b3086b626860d9d4403f310e64dd608a705b610c1222f03d1ef79dcbcf3fd402"
    );
}

#[test]
fn end_user_scope_appends_the_end_user_id() {
    let metadata = object(&[
        ("user_api_key", string("key-hash")),
        ("user_api_key_end_user_id", string("end-user")),
    ]);
    let mut tenant_request = with_params(vec![param("model", string("gpt-4"))]);
    tenant_request.metadata = Some(metadata.clone());
    let mut end_user_request = with_params(vec![param("model", string("gpt-4"))]);
    end_user_request.metadata = Some(metadata);
    let mut end_user_settings = settings_for(LiteLlmCacheType::QdrantSemantic);
    end_user_settings.semantic_cache_scope = SemanticCacheScope::EndUser;
    let tenant_key = derive(
        &mut tenant_request,
        &settings_for(LiteLlmCacheType::QdrantSemantic),
    );
    let end_user_key = derive(&mut end_user_request, &end_user_settings);
    assert_eq!(tenant_key, API_KEY_SCOPE);
    assert_eq!(
        end_user_key,
        "d11fd3c8fe9608cfd039d0ac54dae2a94e3160d929b25ce3b4a007432b835bd4"
    );
}

#[test]
fn tenant_scope_searches_every_metadata_source() {
    let mut nested = with_params(vec![param("model", string("gpt-4"))]);
    let mut params = empty_litellm_params();
    params.metadata = Some(object(&[("user_api_key", string("key-hash"))]));
    nested.litellm_params = Some(params);
    let mut top_level = with_params(vec![param("model", string("gpt-4"))]);
    top_level.litellm_metadata = Some(object(&[("user_api_key", string("key-hash"))]));
    let settings = settings_for(LiteLlmCacheType::RedisSemantic);
    assert_eq!(
        derive(&mut nested, &settings),
        derive(&mut top_level, &settings)
    );
}

#[test]
fn tenant_scope_prefers_the_first_metadata_source() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[("user_api_key", string("first"))]));
    request.litellm_metadata = Some(object(&[("user_api_key", string("second"))]));
    let mut null_request = with_params(vec![param("model", string("gpt-4"))]);
    null_request.metadata = Some(object(&[("user_api_key", KeyValue::Null)]));
    null_request.litellm_metadata = Some(object(&[("user_api_key", string("second"))]));
    let settings = settings_for(LiteLlmCacheType::RedisSemantic);
    assert_eq!(
        derive(&mut request, &settings),
        "fe32ab3e9b588dfbdc5d8e518b869b34cf592026c9fdec0bb04039ab26bae1a5"
    );
    assert_eq!(
        derive(&mut null_request, &settings),
        "c95ead47a44d0dcd8a329f1bdbd46df306034e6d42e7c6822ed1a7d1afef3e92"
    );
}

#[test]
fn tenant_scope_reads_the_litellm_params_litellm_metadata() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    let mut params = empty_litellm_params();
    params.litellm_metadata = Some(object(&[("user_api_key", string("key-hash"))]));
    request.litellm_params = Some(params);
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::RedisSemantic));
    assert_eq!(key, API_KEY_SCOPE);
}

#[test]
fn preset_key_bypasses_derivation() {
    let mut params = empty_litellm_params();
    params.preset_cache_key = Some("given-key".to_owned());
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.litellm_params = Some(params);
    let mut settings = settings_for(LiteLlmCacheType::Local);
    settings.namespace = Some("team-a".to_owned());
    let hooks = hooks(true);
    let key = get_cache_key(&mut request, &settings, &hooks).expect("the preset key");
    assert_eq!(key, "given-key");
    assert_eq!(stored_preset(&request).as_deref(), Some("given-key"));
    assert!(console(&hooks).is_empty());
    assert_eq!(
        logged(&hooks),
        vec![Payload::String(
            "returning preset cache key: given-key".to_owned()
        )]
    );
}

#[test]
fn derived_key_is_stored_for_reuse() {
    let mut request = with_params(vec![
        param("model", string("gpt-4")),
        param("temperature", number(0.7)),
    ]);
    request.litellm_params = Some(empty_litellm_params());
    let settings = settings_for(LiteLlmCacheType::Local);
    let hooks = hooks(false);
    let first = get_cache_key(&mut request, &settings, &hooks).expect("a cache key");
    assert_eq!(first, KEY_AND_TEMPERATURE);
    request.params.push(param("max_tokens", integer(5)));
    let second = get_cache_key(&mut request, &settings, &hooks).expect("the stored preset key");
    assert_eq!(second, first);
    assert_eq!(
        stored_preset(&request).as_deref(),
        Some(KEY_AND_TEMPERATURE)
    );
}

#[test]
fn model_resolves_the_caching_group_first() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[
        ("model_group", string("gpt-4")),
        (
            "caching_groups",
            KeyValue::Array(vec![KeyValue::Array(vec![
                string("gpt-4"),
                string("gpt-4o"),
            ])]),
        ),
    ]));
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(
        key,
        "4c7de5547fcf7673c1327068d7b280e7ee15204008c3862b22510fd14fb884b6"
    );
}

#[test]
fn model_falls_back_to_the_model_group() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[("model_group", string("group-1"))]));
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(
        key,
        "c66b623261d71d26c4b1fa9c755c964a5c7465165b410ef05133c5ea513c9094"
    );
}

#[test]
fn model_group_comes_from_the_litellm_params_metadata() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    let mut params = empty_litellm_params();
    params.metadata = Some(object(&[("model_group", string("group-2"))]));
    request.litellm_params = Some(params);
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(
        key,
        "9427516c0aa63ad7c22ce120c73e2624da4b3531cba13f559794e5a1d2971df7"
    );
}

#[test]
fn model_falls_back_to_the_model_when_the_group_is_null() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[
        ("model_group", KeyValue::Null),
        ("caching_groups", KeyValue::Array(Vec::new())),
    ]));
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(key, MODEL_ONLY);
}

#[test]
fn caching_group_without_a_model_group_keeps_the_model() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[(
        "caching_groups",
        KeyValue::Array(vec![KeyValue::Array(vec![string("gpt-4")])]),
    )]));
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(key, MODEL_ONLY);
}

#[test]
fn malformed_caching_groups_are_rejected() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[("caching_groups", string("gpt-4,gpt-4o"))]));
    let hooks = hooks(false);
    let error = get_cache_key(&mut request, &settings_for(LiteLlmCacheType::Local), &hooks)
        .expect_err("a malformed caching group");
    assert!(matches!(error, CacheKeyError::CachingGroups));
}

#[rstest]
#[case::checksum(
    Some(object(&[("file_checksum", string("sha256-abc"))])),
    None,
    string("audio.mp3"),
    "214104201987dd0ae79a87889d77b3ed78912eaa43b85bec96ce233c06a1e48c"
)]
#[case::file_object_name(
    None,
    None,
    string("audio.mp3"),
    "bdbb9837bd9f0dada1fddec3288c1993c666c15636164ab8fa18d8658888ce96"
)]
#[case::file_object_name_before_metadata_file_name(
    Some(object(&[("file_name", string("metadata-name.mp3"))])),
    None,
    string("audio.mp3"),
    "bdbb9837bd9f0dada1fddec3288c1993c666c15636164ab8fa18d8658888ce96"
)]
#[case::metadata_file_name(
    Some(object(&[("file_name", string("metadata-name.mp3"))])),
    None,
    KeyValue::Null,
    "e447f71e9c0d66b789eb92b8b006e37153b2ad095bb4e3530f76f345cb986c2d"
)]
#[case::metadata_file_name_before_params_file_name(
    Some(object(&[("file_name", string("metadata-name.mp3"))])),
    Some(string("params-name.mp3")),
    KeyValue::Null,
    "e447f71e9c0d66b789eb92b8b006e37153b2ad095bb4e3530f76f345cb986c2d"
)]
#[case::litellm_params_file_name(
    None,
    Some(string("params-name.mp3")),
    KeyValue::Null,
    "36e88a6a43bc3d2a7caf7a575dfb4499bf67b324ae6f46c6a9809ed8cd436ce2"
)]
#[case::no_identity(
    None,
    None,
    KeyValue::Null,
    "3cb3312a5845c5d2386c8022a55d18bebc8890fae9e704cf97203da42e301b08"
)]
fn file_identity_uses_the_first_available_source(
    #[case] metadata: Option<KeyObject>,
    #[case] params_file_name: Option<KeyValue>,
    #[case] file_value: KeyValue,
    #[case] expected: &str,
) {
    let mut request = with_params(vec![
        param("model", string("whisper-1")),
        param("file", file_value),
    ]);
    request.metadata = metadata;
    let mut params = empty_litellm_params();
    params.file_name = params_file_name;
    request.litellm_params = Some(params);
    let key = derive(&mut request, &settings_for(LiteLlmCacheType::Local));
    assert_eq!(key, expected);
}

#[test]
fn namespace_prefers_the_request_then_the_metadata_then_the_settings() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.metadata = Some(object(&[("redis_namespace", string("meta-ns"))]));
    request.cache_control = Some(RequestCacheControl {
        namespace: Some("request-ns".to_owned()),
        use_cache: false,
    });
    let mut configured = settings_for(LiteLlmCacheType::Local);
    configured.namespace = Some("team-a".to_owned());
    assert_eq!(
        derive(&mut request, &configured),
        format!("request-ns:{MODEL_ONLY}")
    );
    request.cache_control = None;
    assert_eq!(
        derive(&mut request, &configured),
        format!("meta-ns:{MODEL_ONLY}")
    );
    request.metadata = None;
    assert_eq!(
        derive(&mut request, &configured),
        format!("team-a:{MODEL_ONLY}")
    );
}

#[test]
fn an_empty_request_namespace_prefixes_nothing() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    request.cache_control = Some(RequestCacheControl {
        namespace: Some(String::new()),
        use_cache: false,
    });
    let mut configured = settings_for(LiteLlmCacheType::Local);
    configured.namespace = Some("team-a".to_owned());
    assert_eq!(derive(&mut request, &configured), MODEL_ONLY);
}

#[test]
fn provider_specific_params_join_the_key_only_with_the_flag() {
    let params = vec![
        param("model", string("gpt-4")),
        param("anthropic_beta", string("tools-2024")),
    ];
    let mut flagged_request = with_params(params.clone());
    let mut plain_request = with_params(params);
    let mut flagged = settings_for(LiteLlmCacheType::Local);
    flagged.include_provider_params = true;
    assert_eq!(derive(&mut flagged_request, &flagged), PROVIDER_BETA);
    assert_eq!(
        derive(&mut plain_request, &settings_for(LiteLlmCacheType::Local)),
        MODEL_ONLY
    );
}

#[test]
fn owned_params_never_join_the_key_even_with_the_flag() {
    let mut request = with_params(vec![
        param("model", string("gpt-4")),
        param("api_key", string("sk-secret")),
        param("num_retries", integer(2)),
        param("metadata", KeyValue::Null),
    ]);
    let mut flagged = settings_for(LiteLlmCacheType::Local);
    flagged.include_provider_params = true;
    assert_eq!(derive(&mut request, &flagged), MODEL_ONLY);
}

#[test]
fn null_values_never_join_the_key() {
    let mut request = with_params(vec![
        param("model", string("gpt-4")),
        param("top_k", KeyValue::Null),
        param("anthropic_beta", KeyValue::Null),
    ]);
    let mut flagged = settings_for(LiteLlmCacheType::Local);
    flagged.include_provider_params = true;
    assert_eq!(derive(&mut request, &flagged), MODEL_ONLY);
}

#[test]
fn litellm_params_stay_out_of_the_key_with_the_provider_flag() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    let mut params = empty_litellm_params();
    params.metadata = Some(object(&[("user_api_key", string("key-hash"))]));
    request.litellm_params = Some(params);
    let mut flagged = settings_for(LiteLlmCacheType::Local);
    flagged.include_provider_params = true;
    assert_eq!(derive(&mut request, &flagged), MODEL_ONLY);
}

#[rstest]
#[case::default_on_always(CacheMode::DefaultOn, None, true)]
#[case::default_on_without_opt_in(CacheMode::DefaultOn, Some(false), true)]
#[case::default_off_without_a_request(CacheMode::DefaultOff, None, false)]
#[case::default_off_without_opt_in(CacheMode::DefaultOff, Some(false), false)]
#[case::default_off_with_opt_in(CacheMode::DefaultOff, Some(true), true)]
fn should_use_cache_follows_the_mode_and_the_opt_in(
    #[case] mode: CacheMode,
    #[case] use_cache: Option<bool>,
    #[case] expected: bool,
) {
    let mut settings = settings_for(LiteLlmCacheType::Local);
    settings.mode = mode;
    let control = use_cache.map(|opt_in| RequestCacheControl {
        namespace: None,
        use_cache: opt_in,
    });
    assert_eq!(should_use_cache(&settings, control.as_ref()), expected);
}

#[test]
fn derivation_messages_stay_on_the_debug_log() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    let hooks = hooks(true);
    let key = get_cache_key(&mut request, &settings_for(LiteLlmCacheType::Local), &hooks)
        .expect("a cache key");
    assert!(console(&hooks).is_empty());
    assert_eq!(
        logged(&hooks),
        vec![
            Payload::String(format!("hashed cache key (SHA-256): {key}")),
            Payload::String(format!("final hashed key: {key}")),
            Payload::String(format!(
                "created cache key: {key} (source material length: 12)"
            )),
        ]
    );
}

#[test]
fn derivation_messages_log_without_verbose() {
    let mut request = with_params(vec![param("model", string("gpt-4"))]);
    let hooks = hooks(false);
    let _key = get_cache_key(&mut request, &settings_for(LiteLlmCacheType::Local), &hooks)
        .expect("a cache key");
    assert!(console(&hooks).is_empty());
    assert_eq!(logged(&hooks).len(), 3);
}

#[test]
fn params_decode_from_the_request_json() {
    let params: Vec<KeyParam> = serde_json::from_str(
        r#"[{"name": "temperature", "value": 0.7}, {"name": "max_tokens", "value": 100}, {"name": "stream", "value": true}, {"name": "messages", "value": [{"role": "user"}]}]"#,
    )
    .expect("request params");
    assert_eq!(params.len(), 4);
    assert_eq!(params[0].value, number(0.7));
    assert_eq!(params[1].value, integer(100));
    assert_eq!(params[2].value, KeyValue::Bool(true));
    assert_eq!(
        params[3].value,
        KeyValue::Array(vec![KeyValue::Object(object(&[("role", string("user"))]))])
    );
}

#[test]
fn object_values_keep_their_json_key_order() {
    let value: KeyValue =
        serde_json::from_str(r#"{"type": "json_object", "strict": true}"#).expect("an object");
    assert_eq!(value.render(), "{'type': 'json_object', 'strict': True}");
}

#[rstest]
#[case::string(string("gpt-4"), "gpt-4")]
#[case::true_value(KeyValue::Bool(true), "True")]
#[case::false_value(KeyValue::Bool(false), "False")]
#[case::null(KeyValue::Null, "None")]
#[case::integer(integer(100), "100")]
#[case::float(number(0.7), "0.7")]
#[case::integral_float(number(1.0), "1.0")]
#[case::array(KeyValue::Array(vec![string("a"), string("b")]), "['a', 'b']")]
#[case::object(
    KeyValue::Object(object(&[("type", string("json_object"))])),
    "{'type': 'json_object'}"
)]
#[case::nested_nulls(
    KeyValue::Array(vec![KeyValue::Null, KeyValue::Bool(true)]),
    "[None, True]"
)]
#[case::quote_inside(string("it's"), "it's")]
fn values_render_as_python_str(#[case] value: KeyValue, #[case] expected: &str) {
    assert_eq!(value.render(), expected);
}

#[rstest]
#[case::embedded_quote(
    KeyValue::Object(object(&[("name", string("it's"))])),
    "{'name': \"it's\"}"
)]
#[case::control_character(
    KeyValue::Object(object(&[("name", string("bell\u{7}"))])),
    "{'name': 'bell\\x07'}"
)]
#[case::escapes(
    KeyValue::Object(object(&[("name", string("back\\slash\ntab\t"))])),
    "{'name': 'back\\\\slash\\ntab\\t'}"
)]
#[case::delimiter_inside_both_quotes(
    KeyValue::Object(object(&[("name", string("he said \"it's\""))])),
    "{'name': 'he said \"it\\'s\"'}"
)]
#[case::non_breaking_space(
    KeyValue::Object(object(&[("name", string("a\u{a0}b"))])),
    "{'name': 'a\\xa0b'}"
)]
#[case::zero_width_space(
    KeyValue::Object(object(&[("name", string("a\u{200b}b"))])),
    "{'name': 'a\\u200bb'}"
)]
#[case::line_separator(
    KeyValue::Object(object(&[("name", string("a\u{2028}b"))])),
    "{'name': 'a\\u2028b'}"
)]
#[case::astral_format_character(
    KeyValue::Object(object(&[("name", string("a\u{e0001}b"))])),
    "{'name': 'a\\U000e0001b'}"
)]
#[case::space_stays_printable(
    KeyValue::Object(object(&[("name", string("a b"))])),
    "{'name': 'a b'}"
)]
fn nested_strings_render_as_python_repr(#[case] value: KeyValue, #[case] expected: &str) {
    assert_eq!(value.render(), expected);
}

#[rstest]
#[case::tenth(0.7_f64, "0.7")]
#[case::one(1.0_f64, "1.0")]
#[case::fifteen(15.0_f64, "15.0")]
#[case::hundred(100.0_f64, "100.0")]
#[case::thousand_fraction(1_234.5_f64, "1234.5")]
#[case::ten_thousandths(0.0001_f64, "0.0001")]
#[case::hundred_thousandths(0.000_01_f64, "1e-05")]
#[case::tiny(1e-7_f64, "1e-07")]
#[case::quadrillion(1e15_f64, "1000000000000000.0")]
#[case::ten_quadrillion(1e16_f64, "1e+16")]
#[case::large_fraction(1.5e16_f64, "1.5e+16")]
#[case::large_integral(123_456_789_012_345_680.0_f64, "1.2345678901234568e+17")]
#[case::huge(1e300_f64, "1e+300")]
#[case::tiny_negative(-1e-300_f64, "-1e-300")]
#[case::negative_zero(-0.0_f64, "-0.0")]
#[case::negative(-1.25_f64, "-1.25")]
#[case::denormal(5e-324_f64, "5e-324")]
#[case::long_fraction(1.234_567_890_123_456_7_f64, "1.2345678901234567")]
fn floats_render_as_python_repr(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(number(value).render(), expected);
}

#[rstest]
#[case::local(LiteLlmCacheType::Local, false)]
#[case::redis(LiteLlmCacheType::Redis, false)]
#[case::redis_semantic(LiteLlmCacheType::RedisSemantic, true)]
#[case::valkey_semantic(LiteLlmCacheType::ValkeySemantic, true)]
#[case::s3(LiteLlmCacheType::S3, false)]
#[case::disk(LiteLlmCacheType::Disk, false)]
#[case::qdrant_semantic(LiteLlmCacheType::QdrantSemantic, true)]
#[case::azure_blob(LiteLlmCacheType::AzureBlob, false)]
#[case::gcs(LiteLlmCacheType::Gcs, false)]
fn only_semantic_cache_types_are_semantic(
    #[case] cache_type: LiteLlmCacheType,
    #[case] expected: bool,
) {
    assert_eq!(settings_for(cache_type).is_semantic_cache(), expected);
}

#[rstest]
#[case::default_on(CacheMode::DefaultOn, "default_on")]
#[case::default_off(CacheMode::DefaultOff, "default_off")]
fn cache_mode_wire_values(#[case] mode: CacheMode, #[case] expected: &str) {
    assert_eq!(serde_json::to_value(mode).expect("a wire value"), expected);
}

#[rstest]
#[case::key(SemanticCacheScope::Key, "key")]
#[case::end_user(SemanticCacheScope::EndUser, "end_user")]
fn semantic_scope_wire_values(#[case] scope: SemanticCacheScope, #[case] expected: &str) {
    assert_eq!(serde_json::to_value(scope).expect("a wire value"), expected);
}
