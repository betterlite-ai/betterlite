#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::ocr_pricing::{OcrBilling, OcrResponseKind, OcrUsage};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::usage_sum::Usage;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;

use super::super::calls::vector_store::vector_store_search_cost;
use super::super::error::CallPriceError;
use super::super::policy::error::PolicyError;
use super::super::policy::{
    CharacterPriceRequest, ProviderPricePolicy, RerankPriceRequest, SearchPriceRequest,
    TokenPriceRequest, VectorStorePriceRequest,
};
use super::super::{CallPriceEnv, CallPriceInput, call_price};
use super::{PRICED, costs, dec, fake_pair, input, modality_details, record, rig};
use crate::routes::CallType;

fn speech(provider: &str) -> CallPriceInput<'_> {
    CallPriceInput {
        custom_llm_provider: Some(provider),
        call_type: CallType::Speech,
        prompt_characters: Some(100),
        ..input("openai/tts-1")
    }
}

#[test]
fn vertex_speech_prices_a_lyria_model_first() {
    let mut rig = rig(&[]);
    rig.fake.lyria = Some(dec("0.5"));
    assert_eq!(
        costs(rig.price(&speech("vertex_ai")).unwrap()),
        (dec("0"), dec("0.5"))
    );
    assert_eq!(rig.policy_calls(), vec!["lyria vertex_ai tts-1".to_owned()]);
    assert!(rig.catalog_calls().is_empty());
}

#[test]
fn speech_for_another_provider_never_asks_for_lyria() {
    let mut rig = rig(&[("tts-1", r#"{"input_cost_per_character": 0.0001}"#)]);
    rig.fake.lyria = Some(dec("0.5"));
    rig.price(&speech("openai")).unwrap();
    assert!(rig.policy_calls().is_empty());
}

#[test]
fn character_priced_speech_bills_prompt_characters_with_a_free_completion() {
    let entry = r#"{"input_cost_per_character": 0.0001, "output_cost_per_character": 5}"#;
    let rig = rig(&[("tts-1", entry)]);
    assert_eq!(
        costs(rig.price(&speech("openai")).unwrap()),
        (dec("0.01"), dec("0"))
    );
    assert_eq!(rig.catalog_calls()[0], "info tts-1 openai");
}

#[test]
fn token_priced_speech_uses_generic_token_pricing_under_the_stripped_name() {
    let rig = rig(&[("tts-1", PRICED)]);
    let request = CallPriceInput {
        prompt_tokens: 10,
        ..speech("openai")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), (dec("0.01"), dec("0")));
    assert!(
        rig.catalog_calls()
            .contains(&"priced tts-1 openai".to_owned())
    );
}

#[test]
fn character_speech_needs_prompt_characters() {
    let rig = rig(&[("tts-1", r#"{"input_cost_per_character": 0.0001}"#)]);
    let request = CallPriceInput {
        prompt_characters: None,
        ..speech("openai")
    };
    let error = rig.price(&request).unwrap_err();
    assert!(matches!(
        error,
        CallPriceError::MissingPromptCharacters { .. }
    ));
    assert_eq!(
        error.to_string(),
        "prompt_characters must be provided for tts calls. model=tts-1, custom_llm_provider=openai"
    );
}

#[test]
fn character_speech_with_an_unusable_rate_has_no_cost() {
    let rig = rig(&[("tts-1", r#"{"input_cost_per_character": "free"}"#)]);
    let error = rig.price(&speech("openai")).unwrap_err();
    assert!(matches!(error, CallPriceError::MissingSpeechCost { .. }));
}

#[test]
fn speech_for_an_unknown_model_fails() {
    let error = rig(&[]).price(&speech("openai")).unwrap_err();
    assert!(matches!(error, CallPriceError::Duration(_)));
}

#[test]
fn rerank_delegates_with_or_without_model_info() {
    let rig = rig(&[("rerank-v3", "{}")]);
    let known = CallPriceInput {
        custom_llm_provider: Some("cohere"),
        call_type: CallType::Rerank,
        ..input("rerank-v3")
    };
    assert_eq!(costs(rig.price(&known).unwrap()), fake_pair());
    let unknown = CallPriceInput {
        model: Some("rerank-v4"),
        ..known
    };
    assert_eq!(costs(rig.price(&unknown).unwrap()), fake_pair());
    assert_eq!(
        rig.policy_calls(),
        vec![
            "rerank cohere rerank-v3 false true".to_owned(),
            "rerank cohere rerank-v4 false false".to_owned()
        ]
    );
}

#[test]
fn vector_search_without_a_configuration_is_free() {
    let response = serde_json::from_str(
        r#"{"object": "vector_store.search_results.page", "search_query": "q", "data": []}"#,
    )
    .unwrap();
    let rig = rig(&[]);
    let context = rig.context();
    let env = CallPriceEnv {
        lookup: &rig.catalog,
        policy: &rig.fake,
        resolution: &context,
        database: &TimeZoneDatabase::from_env(),
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    let free = vector_store_search_cost(None, None, &response, &env).unwrap();
    assert_eq!(costs(free), (dec("0"), dec("0")));
    let slashed =
        vector_store_search_cost(Some("deepseek/store"), Some("openai"), &response, &env).unwrap();
    assert_eq!(costs(slashed), (dec("0"), dec("0")));
    assert_eq!(
        rig.policy_calls(),
        vec![
            "vector openai None".to_owned(),
            "vector deepseek Some(\"store\")".to_owned()
        ]
    );
}

#[test]
fn vector_search_returns_the_configured_price() {
    let response = serde_json::from_str(
        r#"{"object": "vector_store.search_results.page", "search_query": "q", "data": []}"#,
    )
    .unwrap();
    let mut rig = rig(&[]);
    rig.fake.vector = Some(GenericCostResult {
        prompt_cost: dec("3"),
        completion_cost: dec("4"),
    });
    let request = CallPriceInput {
        call_type: CallType::VectorStoreSearch,
        vector_response: Some(&response),
        custom_llm_provider: Some("openai"),
        ..input("store")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), (dec("3"), dec("4")));
    let missing = CallPriceInput {
        vector_response: None,
        ..request
    };
    assert!(matches!(
        rig.price(&missing).unwrap_err(),
        CallPriceError::MissingInput { .. }
    ));
}

#[test]
fn ocr_bills_pages_and_leaves_the_completion_free() {
    let rig = rig(&[("ocr-model", r#"{"ocr_cost_per_page": 0.01}"#)]);
    let billing = OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: Some(OcrUsage {
            credits: None,
            pages_processed: Some(3),
            pages_processed_annotation: 0,
        }),
    };
    let request = CallPriceInput {
        custom_llm_provider: Some("mistral"),
        call_type: CallType::Ocr,
        ocr_billing: Some(&billing),
        ..input("ocr-model")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), (dec("0.03"), dec("0")));
    let missing = CallPriceInput {
        ocr_billing: None,
        ..request
    };
    assert!(matches!(
        rig.price(&missing).unwrap_err(),
        CallPriceError::MissingInput { .. }
    ));
}

#[test]
fn batch_retrieval_bills_the_batch_rates_of_the_selected_usage() {
    let entry = r#"{"input_cost_per_token_batches": 0.5, "output_cost_per_token_batches": 1}"#;
    let rig = rig(&[("gpt-4o", entry)]);
    let request = CallPriceInput {
        custom_llm_provider: Some("openai"),
        call_type: CallType::RetrieveBatch,
        prompt_tokens: 100,
        completion_tokens: 10,
        ..input("gpt-4o")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), (dec("50"), dec("10")));
}

#[rstest]
#[case::audio_detail(Some(3), None, 0, 0, true)]
#[case::text_detail(None, Some(3), 0, 0, true)]
#[case::prompt_tokens(None, None, 2, 0, true)]
#[case::completion_tokens(None, None, 0, 2, true)]
#[case::zero_details(Some(0), Some(0), 0, 0, false)]
fn transcription_bills_tokens_only_on_positive_token_evidence(
    #[case] audio: Option<i64>,
    #[case] text: Option<i64>,
    #[case] prompt: i64,
    #[case] completion: i64,
    #[case] tokens: bool,
) {
    let entry = r#"{"input_cost_per_token": 0.001, "output_cost_per_token": 0.002, "input_cost_per_second": 0.1}"#;
    let rig = rig(&[("whisper-1", entry)]);
    let usage = Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.saturating_add(completion),
        prompt_tokens_details: Some(modality_details(audio, text)),
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    };
    let request = CallPriceInput {
        custom_llm_provider: Some("openai"),
        call_type: CallType::Transcription,
        usage: Some(&usage),
        audio_seconds: dec("30"),
        ..input("openai/whisper-1")
    };
    let priced = rig.price(&request).unwrap();
    let calls = rig.catalog_calls();
    assert_eq!(
        calls.contains(&"priced whisper-1 openai".to_owned()),
        tokens
    );
    if !tokens {
        assert_eq!(costs(priced), (dec("3"), dec("0")));
    }
}

#[rstest]
#[case::missing(None, 1)]
#[case::zero(Some(0), 1)]
#[case::counted(Some(3), 3)]
fn search_counts_at_least_one_query(#[case] queries: Option<i64>, #[case] expected: i64) {
    let rig = rig(&[]);
    let hidden = record(r#"{"max_results": 5}"#);
    let request = CallPriceInput {
        custom_llm_provider: Some("tavily"),
        call_type: CallType::Search,
        number_of_queries: queries,
        search_hidden_params: Some(&hidden),
        ..input("tavily/search")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), fake_pair());
    assert_eq!(
        rig.policy_calls(),
        vec![format!("search tavily tavily/search {expected} true")]
    );
}

struct Silent;

impl ProviderPricePolicy for Silent {}

#[test]
fn an_uninstalled_provider_policy_is_a_typed_unavailable_adapter() {
    let rig = rig(&[]);
    let context = rig.context();
    let request = CallPriceInput {
        custom_llm_provider: Some("deepseek"),
        ..input("chat")
    };
    let error = call_price(
        &request,
        &CallPriceEnv {
            lookup: &rig.catalog,
            policy: &Silent,
            resolution: &context,
            database: &TimeZoneDatabase::from_env(),
            moment: DateTime::<Utc>::UNIX_EPOCH,
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CallPriceError::Policy(PolicyError::UnavailableAdapter(provider)) if provider == "deepseek"
    ));
}

fn unavailable(error: PolicyError) -> String {
    match error {
        PolicyError::UnavailableAdapter(provider) => provider,
    }
}

#[test]
fn default_token_and_character_policies_report_their_provider() {
    let usage = CostUsage::from(&Usage::default());
    let extras = PromptExtras::default();
    let token = TokenPriceRequest {
        provider: "p-token",
        model: "m",
        usage: &usage,
        prompt_extras: &extras,
        service_tier: None,
        vertex_location: None,
    };
    let character = CharacterPriceRequest {
        provider: "p-character",
        model: "m",
        prompt_characters: None,
        completion_characters: None,
        usage: &usage,
        prompt_extras: &extras,
        service_tier: None,
        vertex_location: None,
    };
    assert_eq!(
        unavailable(Silent.token_cost(&token).unwrap_err()),
        "p-token"
    );
    assert_eq!(
        unavailable(Silent.character_cost(&character).unwrap_err()),
        "p-character"
    );
}

#[test]
fn default_search_rerank_vector_lyria_and_route_policies_report_their_provider() {
    let search = SearchPriceRequest {
        provider: "p-search",
        model: "m",
        number_of_queries: 1,
        hidden_params: None,
    };
    let rerank = RerankPriceRequest {
        provider: "p-rerank",
        model: "m",
        billed_units: None,
        model_info: None,
    };
    let response =
        serde_json::from_str(r#"{"object": "o", "search_query": "q", "data": []}"#).unwrap();
    let vector = VectorStorePriceRequest {
        provider: "p-vector",
        api_type: None,
        response: &response,
    };
    assert_eq!(
        unavailable(Silent.search_cost(&search).unwrap_err()),
        "p-search"
    );
    assert_eq!(
        unavailable(Silent.rerank_cost(&rerank).unwrap_err()),
        "p-rerank"
    );
    assert_eq!(
        unavailable(Silent.vector_store_cost(&vector).unwrap_err()),
        "p-vector"
    );
    assert_eq!(
        unavailable(Silent.lyria_cost("p-lyria", "m").unwrap_err()),
        "p-lyria"
    );
    assert_eq!(
        unavailable(Silent.vertex_route("m", CallType::Completion).unwrap_err()),
        "vertex_ai"
    );
}
