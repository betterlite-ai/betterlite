#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_error::error::ErrorClass;
use betterlite_error::facts::ProviderErrorFacts;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::metadata::identity::AudioFormat;
use betterlite_models::metadata::vendor::VertexAudioApi;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::model_policy::UNSUPPORTED_DEPLOYED_PARAMS;
use crate::model_policy::capabilities::response_schema::{
    get_supports_response_schema, supports_response_json_schema,
};
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
    get_supports_system_message,
};
use crate::model_policy::error::{VertexAiError, classify_vertex_error};
use crate::model_policy::lyria::{
    LyriaModelInfo, LyriaOverlay, lyria_model_key, validate_lyria_model_info,
};
use crate::model_policy::partners::{is_vertex_partner_model, should_use_openai_handler};
use crate::model_policy::publisher::is_self_deployed_endpoint;
use crate::model_policy::route::{RouteParams, get_vertex_ai_model_route};
use crate::model_urls::route::VertexModelRoute;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

#[rstest]
#[case::base_model_gemini_first(
    "agent_engine/7",
    Some("gemini-1.5-pro"),
    None,
    VertexModelRoute::Gemini
)]
#[case::base_model_matches_substring(
    "claude-3",
    Some("not-gemini"),
    None,
    VertexModelRoute::Gemini
)]
#[case::base_model_empty_keeps_model_route(
    "agent_engine/7",
    Some(""),
    None,
    VertexModelRoute::AgentEngine
)]
#[case::base_model_without_gemini_falls_through(
    "meta/llama-3",
    Some("bge"),
    None,
    VertexModelRoute::PartnerModels
)]
#[case::agent_engine("agent_engine/abc", None, None, VertexModelRoute::AgentEngine)]
#[case::agent_engine_wins_over_numeric(
    "agent_engine/123",
    None,
    Some("http://host"),
    VertexModelRoute::AgentEngine
)]
#[case::numeric_psc_endpoint(
    "1234567890",
    None,
    Some("http://10.96.32.8"),
    VertexModelRoute::Gemini
)]
#[case::numeric_without_api_base("1234567890", None, None, VertexModelRoute::NonGemini)]
#[case::numeric_with_empty_api_base("1234567890", None, Some(""), VertexModelRoute::NonGemini)]
#[case::numeric_leading_zero("0042", None, Some("https://host/v1"), VertexModelRoute::Gemini)]
#[case::numeric_base_model_beats_api_base(
    "999",
    Some("not-gemini"),
    Some("https://host"),
    VertexModelRoute::Gemini
)]
#[case::numeric_prefixed_model("bge/123", None, Some("https://host"), VertexModelRoute::Bge)]
#[case::partner_gemma_maas("google/gemma-3-12b-it", None, None, VertexModelRoute::PartnerModels)]
#[case::partner_prefix_before_gemma("google/gemma-3/", None, None, VertexModelRoute::PartnerModels)]
#[case::partner_gpt_oss(
    "openai/gpt-oss-120b-maas",
    None,
    None,
    VertexModelRoute::PartnerModels
)]
#[case::partner_meta(
    "meta/llama-3.1-405b-instruct-maas",
    None,
    None,
    VertexModelRoute::PartnerModels
)]
#[case::partner_claude("claude-3-5-sonnet", None, None, VertexModelRoute::PartnerModels)]
#[case::deployed_gemma("gemma/gemma-3-12b-it", None, None, VertexModelRoute::Gemma)]
#[case::deployed_gemma_numeric("gemma/123456", None, None, VertexModelRoute::Gemma)]
#[case::bge_prefix("bge/378943383978115072", None, None, VertexModelRoute::Bge)]
#[case::bge_substring("mybge-model", None, None, VertexModelRoute::Bge)]
#[case::bge_uppercase("BGE/model", None, None, VertexModelRoute::Bge)]
#[case::gemma_without_slash("gemma-3-12b-it", None, None, VertexModelRoute::NonGemini)]
#[case::model_garden_openai("openai/gpt-4", None, None, VertexModelRoute::ModelGarden)]
#[case::model_garden_xai("xai/grok-4.1-fast", None, None, VertexModelRoute::ModelGarden)]
#[case::model_garden_uppercase_xai("XAI/grok-4.1-fast", None, None, VertexModelRoute::NonGemini)]
#[case::gemini_model("gemini-2.5-pro", None, None, VertexModelRoute::Gemini)]
#[case::gemini_uppercase("GEMINI-2.5-PRO", None, None, VertexModelRoute::NonGemini)]
#[case::gemini_spec_prefix("gemini/1234", None, None, VertexModelRoute::Gemini)]
#[case::legacy_model("chat-bison", None, None, VertexModelRoute::NonGemini)]
#[case::empty_model("", None, Some("https://host"), VertexModelRoute::NonGemini)]
#[case::route_prefix_model_garden("model_garden/x", None, None, VertexModelRoute::NonGemini)]
fn route_cases(
    #[case] model: &str,
    #[case] base_model: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] expected: VertexModelRoute,
) {
    let params = RouteParams {
        base_model,
        api_base,
    };
    assert_eq!(
        get_vertex_ai_model_route(model, &params),
        expected,
        "model {model}"
    );
}

#[test]
fn route_without_params_uses_only_the_model() {
    let params = RouteParams::default();
    assert_eq!(
        get_vertex_ai_model_route("1234567890", &params),
        VertexModelRoute::NonGemini
    );
    assert_eq!(
        get_vertex_ai_model_route("gemini-2.5-pro", &params),
        VertexModelRoute::Gemini
    );
}

#[rstest]
#[case::google_gemma_is_partner("google/gemma-3-12b-it", true, true, false)]
#[case::gemma_slash_is_deployed("gemma/gemma-3-12b-it", false, false, true)]
#[case::gemma_numeric_is_deployed("gemma/123456", false, false, true)]
#[case::bare_gemma_is_not_deployed("gemma-3-12b-it", false, false, false)]
#[case::meta_partner("meta/llama-3", true, true, false)]
#[case::bare_llama_is_openai_handler_only("llama3-405b", false, true, false)]
#[case::gpt_oss_partner("openai/gpt-oss-120b", true, true, false)]
#[case::openai_publisher_is_deployed("openai/foo", false, false, true)]
#[case::openai_nested_publisher("openai/foo/bar", false, false, false)]
#[case::xai_publisher("xai/grok-4.1", false, false, false)]
#[case::dbx_partner("deepseek-ai/DeepSeek-R1", true, true, false)]
#[case::mistral_partner("mistral-large", true, false, false)]
#[case::codestral_partner("codestral-2501", true, false, false)]
#[case::jamba_partner("jamba-1.5", true, false, false)]
#[case::claude_partner("claude-3-5-sonnet", true, false, false)]
#[case::qwen_partner("qwen/qwen3", true, true, false)]
#[case::minimax_partner("minimaxai/minimax-m2", true, true, false)]
#[case::moonshot_partner("moonshotai/kimi-k2", true, true, false)]
#[case::zai_partner("zai-org/glm-4.5", true, true, false)]
#[case::bge_endpoint("bge/123", false, false, false)]
#[case::vertex_prefixed_gemma("vertex_ai/gemma/gemma-3-12b-it", false, false, true)]
#[case::vertex_prefixed_openai("vertex_ai/openai/foo", false, false, true)]
#[case::vertex_prefixed_xai("vertex_ai/xai/grok-4.1", false, false, false)]
#[case::gemini_model("gemini-2.5-pro", false, false, false)]
fn partner_cases(
    #[case] model: &str,
    #[case] partner: bool,
    #[case] openai_handler: bool,
    #[case] self_deployed: bool,
) {
    assert_eq!(is_vertex_partner_model(model), partner, "partner {model}");
    assert_eq!(
        should_use_openai_handler(model),
        openai_handler,
        "openai handler {model}"
    );
    assert_eq!(
        is_self_deployed_endpoint(model),
        self_deployed,
        "self deployed {model}"
    );
}

#[derive(Clone, Copy)]
enum LookupOutcome {
    True,
    False,
    Error,
}

impl LookupOutcome {
    fn value(self) -> Result<bool, CapabilityLookupError> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::Error => Err(CapabilityLookupError {
                message: "capability lookup failed".to_owned(),
            }),
        }
    }
}

struct FixtureLookup {
    system: LookupOutcome,
    schema: LookupOutcome,
    provider: &'static str,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        self.check_provider(provider)?;
        let outcome = match kind {
            CapabilityKind::SystemMessages => self.system,
            CapabilityKind::ResponseSchema => self.schema,
        };
        outcome.value()
    }
}

impl FixtureLookup {
    fn check_provider(&self, provider: &str) -> Result<(), CapabilityLookupError> {
        if provider == self.provider {
            return Ok(());
        }
        Err(CapabilityLookupError {
            message: format!("expected provider {}", self.provider),
        })
    }
}

enum CapabilityCase {
    System {
        model: &'static str,
        provider: VertexCapabilityProvider,
        lookup_provider: &'static str,
        outcome: LookupOutcome,
        expected: bool,
    },
    Schema {
        model: &'static str,
        provider: VertexCapabilityProvider,
        lookup_provider: &'static str,
        outcome: LookupOutcome,
        expected: Option<bool>,
    },
}

#[rstest]
#[case::system_vertex_ai(CapabilityCase::System {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::True,
    expected: true,
})]
#[case::system_vertex_ai_beta_looks_up_vertex_ai(CapabilityCase::System {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::VertexAiBeta,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::True,
    expected: true,
})]
#[case::system_gemini_looks_up_gemini(CapabilityCase::System {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::Gemini,
    lookup_provider: "gemini",
    outcome: LookupOutcome::True,
    expected: true,
})]
#[case::system_unsupported_model_is_false(CapabilityCase::System {
    model: "gemini-1.5-pro",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::False,
    expected: false,
})]
#[case::system_lookup_error_warns_and_returns_false(CapabilityCase::System {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::Error,
    expected: false,
})]
#[case::system_gemini_spec_model_overrides_false(CapabilityCase::System {
    model: "gemini/1234",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::False,
    expected: true,
})]
#[case::system_gemini_spec_model_keeps_true(CapabilityCase::System {
    model: "gemini/1234",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::True,
    expected: true,
})]
#[case::schema_vertex_ai_supported(CapabilityCase::Schema {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::True,
    expected: Some(true),
})]
#[case::schema_vertex_ai_unsupported(CapabilityCase::Schema {
    model: "gemini-1.5-pro",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::False,
    expected: Some(false),
})]
#[case::schema_vertex_ai_beta_looks_up_vertex_ai(CapabilityCase::Schema {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::VertexAiBeta,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::True,
    expected: Some(true),
})]
#[case::schema_gemini_looks_up_gemini(CapabilityCase::Schema {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::Gemini,
    lookup_provider: "gemini",
    outcome: LookupOutcome::True,
    expected: Some(true),
})]
#[case::schema_lookup_error_propagates(CapabilityCase::Schema {
    model: "gemini-2.5-pro",
    provider: VertexCapabilityProvider::VertexAi,
    lookup_provider: "vertex_ai",
    outcome: LookupOutcome::Error,
    expected: None,
})]
fn capability_cases(#[case] case: CapabilityCase) {
    match case {
        CapabilityCase::System {
            model,
            provider,
            lookup_provider,
            outcome,
            expected,
        } => {
            let lookup = FixtureLookup {
                system: outcome,
                schema: LookupOutcome::False,
                provider: lookup_provider,
            };
            assert_eq!(
                get_supports_system_message(model, provider, &lookup),
                expected,
                "model {model}"
            );
        }
        CapabilityCase::Schema {
            model,
            provider,
            lookup_provider,
            outcome,
            expected,
        } => {
            let lookup = FixtureLookup {
                system: LookupOutcome::False,
                schema: outcome,
                provider: lookup_provider,
            };
            let result = get_supports_response_schema(model, provider, &lookup);
            assert_eq!(result.ok(), expected, "model {model}");
        }
    }
}

#[rstest]
#[case::gemini_2("gemini-2.0-flash", true)]
#[case::gemini_2_5("gemini-2.5-pro", true)]
#[case::gemini_3("gemini-3-pro-preview", true)]
#[case::gemini_10("gemini-10-pro", true)]
#[case::gemini_1_5("gemini-1.5-pro", false)]
#[case::gemini_2_without_separator("gemini-2x", false)]
#[case::gemini_20_without_separator("gemini-20", false)]
#[case::gemini_20_with_separator("gemini-20-pro", true)]
#[case::uppercase("GEMINI-2.5-PRO", true)]
#[case::gemma("gemma-2.5-pro", false)]
#[case::prefixed("vertex_ai/gemini-2.5-pro", true)]
fn json_schema_cases(#[case] model: &str, #[case] expected: bool) {
    assert_eq!(
        supports_response_json_schema(model),
        expected,
        "model {model}"
    );
}

#[rstest]
#[case::audio("audio")]
#[case::max_retries("max_retries")]
#[case::modalities("modalities")]
#[case::prediction("prediction")]
#[case::prompt_cache_key("prompt_cache_key")]
#[case::prompt_cache_retention("prompt_cache_retention")]
#[case::safety_identifier("safety_identifier")]
#[case::service_tier("service_tier")]
#[case::store("store")]
#[case::web_search_options("web_search_options")]
fn unsupported_param_cases(#[case] param: &str) {
    assert!(UNSUPPORTED_DEPLOYED_PARAMS.contains(&param));
}

#[test]
fn unsupported_params_are_exactly_the_source_set() {
    assert_eq!(
        UNSUPPORTED_DEPLOYED_PARAMS,
        [
            "audio",
            "max_retries",
            "modalities",
            "prediction",
            "prompt_cache_key",
            "prompt_cache_retention",
            "safety_identifier",
            "service_tier",
            "store",
            "web_search_options",
        ]
    );
    assert!(!UNSUPPORTED_DEPLOYED_PARAMS.contains(&"temperature"));
}

fn bundled_catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn lyria_overlay<'sources>(
    runtime: &'sources BTreeMap<String, Json>,
    bundled: &'sources ModelCatalog,
) -> LyriaOverlay<'sources> {
    LyriaOverlay { runtime, bundled }
}

#[test]
fn lyria_bundled_info_and_cost() {
    let catalog = bundled_catalog();
    let runtime = BTreeMap::new();
    let overlay = lyria_overlay(&runtime, catalog.as_ref());
    assert_eq!(
        overlay.model_info("lyria-002"),
        Some(LyriaModelInfo {
            vertex_ai_audio_api: VertexAudioApi::LyriaPredict,
            supported_audio_formats: vec![AudioFormat::Wav],
            output_cost_per_image: Some(decimal("0.06")),
        })
    );
    assert_eq!(overlay.generation_cost("lyria-002"), Some(decimal("0.06")));
    assert_eq!(
        overlay.model_info("lyria-3-pro-preview"),
        Some(LyriaModelInfo {
            vertex_ai_audio_api: VertexAudioApi::LyriaInteractions,
            supported_audio_formats: vec![AudioFormat::Mp3, AudioFormat::Wav],
            output_cost_per_image: Some(decimal("0.08")),
        })
    );
}

#[test]
fn lyria_invalid_runtime_entry_falls_back_to_bundled() {
    let catalog = bundled_catalog();
    let runtime = BTreeMap::from([(
        "vertex_ai/lyria-002".to_owned(),
        json(
            r#"{"vertex_ai_audio_api": "lyria_unknown", "supported_audio_formats": ["wav"], "output_cost_per_image": 9.5}"#,
        ),
    )]);
    let overlay = lyria_overlay(&runtime, catalog.as_ref());
    let info = overlay.model_info("lyria-002").unwrap();
    assert_eq!(info.vertex_ai_audio_api, VertexAudioApi::LyriaPredict);
    assert_eq!(info.output_cost_per_image, Some(decimal("0.06")));
    assert_eq!(overlay.generation_cost("lyria-002"), Some(decimal("0.06")));
}

#[test]
fn lyria_runtime_info_without_cost_keeps_bundled_cost() {
    let catalog = bundled_catalog();
    let runtime = BTreeMap::from([(
        "vertex_ai/lyria-002".to_owned(),
        json(
            r#"{"vertex_ai_audio_api": "lyria_interactions", "supported_audio_formats": ["mp3", "wav"]}"#,
        ),
    )]);
    let overlay = lyria_overlay(&runtime, catalog.as_ref());
    let info = overlay.model_info("lyria-002").unwrap();
    assert_eq!(info.vertex_ai_audio_api, VertexAudioApi::LyriaInteractions);
    assert_eq!(info.output_cost_per_image, None);
    assert_eq!(overlay.generation_cost("lyria-002"), Some(decimal("0.06")));
}

#[test]
fn lyria_runtime_cost_wins_over_bundled() {
    let catalog = bundled_catalog();
    let runtime = BTreeMap::from([(
        "vertex_ai/lyria-002".to_owned(),
        json(
            r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": 1.5}"#,
        ),
    )]);
    let overlay = lyria_overlay(&runtime, catalog.as_ref());
    assert_eq!(
        overlay
            .model_info("lyria-002")
            .unwrap()
            .output_cost_per_image,
        Some(decimal("1.5"))
    );
    assert_eq!(overlay.generation_cost("lyria-002"), Some(decimal("1.5")));
}

#[test]
fn lyria_runtime_only_model_has_no_bundled_cost() {
    let catalog = bundled_catalog();
    let runtime = BTreeMap::from([(
        "vertex_ai/custom-lyria".to_owned(),
        json(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"]}"#),
    )]);
    let overlay = lyria_overlay(&runtime, catalog.as_ref());
    assert_eq!(
        overlay
            .model_info("custom-lyria")
            .unwrap()
            .vertex_ai_audio_api,
        VertexAudioApi::LyriaPredict
    );
    assert_eq!(overlay.generation_cost("custom-lyria"), None);
}

#[test]
fn lyria_model_without_any_entry_has_no_info() {
    let catalog = bundled_catalog();
    let runtime = BTreeMap::new();
    let overlay = lyria_overlay(&runtime, catalog.as_ref());
    assert_eq!(overlay.model_info("vertex_ai_beta/lyria-002"), None);
    assert_eq!(overlay.generation_cost("vertex_ai_beta/lyria-002"), None);
}

#[rstest]
#[case::numeric_string(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": "0.5"}"#, "0.5")]
#[case::bool_true(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": true}"#, "1")]
#[case::bool_false(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": false}"#, "0")]
#[case::integer(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": 0}"#, "0")]
#[case::exponent(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": 4e-7}"#, "0.0000004")]
fn lyria_runtime_cost_accepts_lax_numbers(#[case] raw: &str, #[case] expected: &str) {
    let info = validate_lyria_model_info(&json(raw)).unwrap();
    assert_eq!(info.output_cost_per_image, Some(decimal(expected)));
}

#[rstest]
#[case::not_an_object(r#""lyria_predict""#)]
#[case::missing_audio_api(r#"{"supported_audio_formats": ["wav"]}"#)]
#[case::unknown_audio_api(
    r#"{"vertex_ai_audio_api": "lyria_other", "supported_audio_formats": ["wav"]}"#
)]
#[case::missing_formats(r#"{"vertex_ai_audio_api": "lyria_predict"}"#)]
#[case::formats_not_a_list(
    r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": "wav"}"#
)]
#[case::unsupported_format(
    r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["ogg"]}"#
)]
#[case::null_cost_invalidates_entry(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": null}"#)]
#[case::list_cost_invalidates_entry(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": [1]}"#)]
#[case::non_numeric_string_cost(r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "output_cost_per_image": "cheap"}"#)]
fn lyria_invalid_entries(#[case] raw: &str) {
    assert_eq!(validate_lyria_model_info(&json(raw)), None, "raw {raw}");
}

#[test]
fn lyria_extra_keys_are_ignored() {
    let info = validate_lyria_model_info(&json(
        r#"{"vertex_ai_audio_api": "lyria_predict", "supported_audio_formats": ["wav"], "extra": 1}"#,
    ));
    assert!(info.is_some());
}

#[test]
fn lyria_key_prefixes_only_a_missing_vertex_ai_prefix() {
    assert_eq!(lyria_model_key("lyria-002"), "vertex_ai/lyria-002");
    assert_eq!(
        lyria_model_key("vertex_ai/lyria-002"),
        "vertex_ai/lyria-002"
    );
    assert_eq!(
        lyria_model_key("vertex_ai_beta/lyria-002"),
        "vertex_ai/vertex_ai_beta/lyria-002"
    );
}

#[test]
fn vertex_error_keeps_status_and_headers() {
    let headers = BTreeMap::from([("retry-after".to_owned(), "7".to_owned())]);
    let error = VertexAiError {
        status_code: 429,
        message: "429 Quota exceeded for quota metric".to_owned(),
        headers: Some(headers.clone()),
    };
    let facts = ProviderErrorFacts::from(&error);
    assert_eq!(facts.status_code, Some(429));
    assert_eq!(facts.headers, Some(headers));
    let class = classify_vertex_error(&error, "vertex_ai", Some("gemini-2.5-pro".to_owned()));
    let Some(ErrorClass::RateLimit(details)) = class else {
        panic!("expected a rate limit class");
    };
    assert_eq!(details.context.provider.as_deref(), Some("vertex_ai"));
    assert_eq!(details.context.model.as_deref(), Some("gemini-2.5-pro"));
}

#[test]
fn vertex_error_falls_back_to_the_status_classifier() {
    let error = VertexAiError {
        status_code: 503,
        message: "plain failure".to_owned(),
        headers: None,
    };
    assert!(matches!(
        classify_vertex_error(&error, "vertex_ai", None),
        Some(ErrorClass::ServiceUnavailable(_))
    ));
}

#[test]
fn vertex_error_with_an_unknown_status_is_unclassified() {
    let error = VertexAiError {
        status_code: 418,
        message: "plain failure".to_owned(),
        headers: None,
    };
    assert_eq!(classify_vertex_error(&error, "vertex_ai", None), None);
}
