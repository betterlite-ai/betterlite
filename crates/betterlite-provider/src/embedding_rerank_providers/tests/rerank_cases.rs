use std::collections::BTreeMap;

use betterlite_config::sections::{cohere_rerank_settings, jina_infinity_rerank};
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::host_rerank::error::HostRerankError;
use rstest::rstest;

use super::*;
use crate::api::rerank_contracts::{
    Headers as RerankHeaders, Params, RerankLogging, RerankProviderResponse, ResponseTransform,
};
use crate::operation::OperationError;
#[test]
fn cohere_rerank_uses_the_v1_client_for_a_v1_base() {
    let mut fixture = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    fixture.setup.api_base = Some(String::from("https://co.test/v1/rerank"));
    fixture.options.max_chunks_per_doc = Some(2);
    fixture.options.top_n = Some(1);
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://co.test/v1/rerank");
    let body = body_of(&prepared);
    assert_eq!(text_field(&body, "model").as_deref(), Some("rerank-v3.5"));
    assert_eq!(text_field(&body, "query").as_deref(), Some("what"));
    assert_eq!(items_field(&body, "documents")[0].as_str(), Some("doc"));
    assert_eq!(number_field(&body, "top_n"), Some(1));
    assert_eq!(flag_field(&body, "return_documents"), Some(true));
    assert_eq!(number_field(&body, "max_chunks_per_doc"), Some(2));
    assert!(body.get("max_tokens_per_doc").is_none());
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer key")
    );
}

#[test]
fn cohere_rerank_v2_is_the_default() {
    let mut fixture = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    fixture.options.max_tokens_per_doc = Some(8);
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://api.cohere.ai/v2/rerank");
    let body = body_of(&prepared);
    assert_eq!(number_field(&body, "max_tokens_per_doc"), Some(8));
    assert!(body.get("max_chunks_per_doc").is_none());
}

#[test]
fn a_legacy_limit_selects_the_v1_client_and_its_suffix_quirk() {
    let mut legacy = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    legacy.setup.api_base = Some(String::from("https://co.test"));
    legacy.options.max_chunks_per_doc = Some(1);
    assert_eq!(url_of(&legacy.prepared()), "https://co.test/v1/rerank");

    let mut both = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    both.setup.api_base = Some(String::from("https://co.test"));
    both.options.max_chunks_per_doc = Some(1);
    both.options.max_tokens_per_doc = Some(1);
    assert_eq!(url_of(&both.prepared()), "https://co.test/v2/rerank");

    let mut v2_base = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    v2_base.setup.api_base = Some(String::from("https://co.test/v2/rerank"));
    v2_base.options.max_chunks_per_doc = Some(1);
    assert_eq!(url_of(&v2_base.prepared()), "https://co.test/v2/rerank");

    // the trailing slash hides the suffix from the selector, so the legacy limit picks v1
    let mut trailing = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    trailing.setup.api_base = Some(String::from("https://co.test/v2/rerank/"));
    trailing.options.max_chunks_per_doc = Some(1);
    assert_eq!(
        url_of(&trailing.prepared()),
        "https://co.test/v2/rerank/v1/rerank"
    );
}

#[test]
fn rank_fields_reach_the_cohere_body() {
    let mut fixture = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    fixture.options.rank_fields = Some(vec![String::from("title")]);
    let body = body_of(&fixture.prepared());
    assert_eq!(items_field(&body, "rank_fields")[0].as_str(), Some("title"));
}

#[test]
fn jina_rerank_sends_its_own_headers_and_params() {
    let mut fixture = RerankFixture::new(ProviderId::JinaAi, "jina-reranker-v2");
    fixture.setup.api_base = Some(String::from("https://jina.test"));
    fixture.setup.headers = Some(headers(&[("x-call", "1")]));
    fixture.options.top_n = Some(2);
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://jina.test/v1/rerank");
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer key")
    );
    assert_eq!(header_of(&prepared, "x-call"), None);
    let body = body_of(&prepared);
    assert_eq!(
        text_field(&body, "model").as_deref(),
        Some("jina-reranker-v2")
    );
    assert_eq!(text_field(&body, "query").as_deref(), Some("what"));
    assert_eq!(number_field(&body, "top_n"), Some(2));
    assert_eq!(items_field(&body, "documents")[0].as_str(), Some("doc"));
}

#[test]
fn infinity_rerank_requires_a_base() {
    let fixture = RerankFixture::new(ProviderId::Infinity, "infinity-rerank");
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::HostRerank(HostRerankError::MissingApiBase))
    ));
}

#[test]
fn voyage_rerank_sends_its_typed_body() {
    let mut fixture = RerankFixture::new(ProviderId::Voyage, "rerank-2");
    fixture.options.top_n = Some(2);
    fixture.options.return_documents = Some(ReturnDocuments::Value(false));
    let prepared = fixture.prepared();
    assert_eq!(url_of(&prepared), "https://api.voyageai.com/v1/rerank");
    assert_eq!(
        header_of(&prepared, "authorization").as_deref(),
        Some("Bearer key")
    );
    assert_eq!(
        header_of(&prepared, "content-type").as_deref(),
        Some("application/json")
    );
    let body = body_of(&prepared);
    assert_eq!(text_field(&body, "model").as_deref(), Some("rerank-2"));
    assert_eq!(text_field(&body, "query").as_deref(), Some("what"));
    assert_eq!(items_field(&body, "documents")[0].as_str(), Some("doc"));
    assert_eq!(number_field(&body, "top_k"), Some(2));
    assert_eq!(flag_field(&body, "return_documents"), Some(false));
}

#[test]
fn voyage_rerank_reads_the_key_from_the_environment() {
    let mut fixture = RerankFixture::new(ProviderId::Voyage, "rerank-2");
    fixture.setup.api_key = None;
    fixture
        .setup
        .environment
        .insert(String::from("VOYAGE_API_KEY"), String::from("env-key"));
    assert_eq!(
        header_of(&fixture.prepared(), "authorization").as_deref(),
        Some("Bearer env-key")
    );
    fixture.setup.environment.clear();
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::VoyageRerank(_))
    ));
}

#[test]
fn an_unsupported_provider_prepares_no_rerank() {
    let fixture = RerankFixture::new(ProviderId::Openai, "rerank");
    assert!(matches!(
        fixture.prepare(),
        Err(FamilyError::Unsupported { .. })
    ));
}
#[test]
fn a_cohere_rerank_reply_is_decoded() {
    let prepared = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5").prepared();
    let answer = embedding_result(decode(&prepared, 200, RERANK_REPLY).unwrap());
    let EmbeddingRerankResult::Rerank(reply) = answer else {
        panic!("the rerank answer");
    };
    let results = reply.results.unwrap();
    assert_eq!(results[0].index, 0);
    assert_eq!(results[0].document.as_ref().unwrap().text, "doc");
    assert_eq!(results[0].relevance_score.0.to_string(), "0.5");
}

#[test]
fn a_jina_rerank_reply_is_decoded() {
    let prepared = RerankFixture::new(ProviderId::JinaAi, "jina-reranker-v2").prepared();
    let reply = r#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5,"document":{"text":"doc"}}],"usage":{"total_tokens":2}}"#;
    let answer = embedding_result(decode(&prepared, 200, reply).unwrap());
    let EmbeddingRerankResult::Rerank(decoded) = answer else {
        panic!("the rerank answer");
    };
    let results = decoded.results.unwrap();
    assert_eq!(results[0].document.as_ref().unwrap().text, "doc");
}

#[test]
fn an_infinity_rerank_reply_is_decoded() {
    let mut fixture = RerankFixture::new(ProviderId::Infinity, "infinity-rerank");
    fixture.setup.api_base = Some(String::from("https://infinity.test"));
    let prepared = fixture.prepared();
    let reply = r#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5}]}"#;
    let answer = embedding_result(decode(&prepared, 200, reply).unwrap());
    let EmbeddingRerankResult::Rerank(decoded) = answer else {
        panic!("the rerank answer");
    };
    let results = decoded.results.unwrap();
    assert_eq!(results[0].index, 0);
    assert!(results[0].document.is_none());
}

#[test]
fn a_voyage_rerank_reply_is_rewritten() {
    let prepared = RerankFixture::new(ProviderId::Voyage, "rerank-2").prepared();
    let answer = embedding_result(decode(&prepared, 200, VOYAGE_RERANK_REPLY).unwrap());
    let EmbeddingRerankResult::Rerank(reply) = answer else {
        panic!("the rerank answer");
    };
    assert_eq!(reply.id.as_deref(), Some("r-1"));
    let results = reply.results.unwrap();
    assert_eq!(results[0].document.as_ref().unwrap().text, "doc");
    let meta = reply.meta.unwrap();
    let billed = meta.billed_units.unwrap().unwrap();
    assert_eq!(billed.total_tokens, Some(Some(3)));
    let tokens = meta.tokens.unwrap().unwrap();
    assert_eq!(tokens.input_tokens, Some(Some(3)));
    assert_eq!(tokens.output_tokens, Some(Some(0)));
}

#[rstest]
#[case::text_is_a_number(
    r#"{"id":"r-1","data":[{"index":0,"relevance_score":0.5,"document":{"text":5}}],"usage":{"total_tokens":3}}"#
)]
#[case::no_text_key(
    r#"{"id":"r-1","data":[{"index":0,"relevance_score":0.5,"document":{"page":3}}],"usage":{"total_tokens":3}}"#
)]
#[case::text_with_another_key(
    r#"{"id":"r-1","data":[{"index":0,"relevance_score":0.5,"document":{"text":"doc","page":3}}],"usage":{"total_tokens":3}}"#
)]
#[case::a_list_document(
    r#"{"id":"r-1","data":[{"index":0,"relevance_score":0.5,"document":[1]}],"usage":{"total_tokens":3}}"#
)]
fn a_voyage_document_without_a_text_string_is_refused(#[case] body: &str) {
    assert!(matches!(
        rerank::decode(RerankConfig::Voyage, 200, body.as_bytes()),
        Err(FamilyError::VoyageDocument)
    ));
}

#[test]
fn a_status_answer_is_the_provider_error() {
    let prepared = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5").prepared();
    let Err(error) = decode(&prepared, 500, "boom") else {
        panic!("a status answer is an error");
    };
    assert!(matches!(
        error,
        OperationError::Provider { response } if response.status == 500
    ));
}

#[test]
fn a_reply_that_is_not_the_family_json_is_a_decode_error() {
    let prepared = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5").prepared();
    let Err(error) = decode(&prepared, 200, "not json") else {
        panic!("a reply that is not json is an error");
    };
    assert!(matches!(
        error,
        OperationError::Decode {
            operation: "embedding_rerank",
            ..
        }
    ));
}

const COHERE_V1_PARAMS: [&str; 6] = [
    "query",
    "documents",
    "top_n",
    "max_chunks_per_doc",
    "rank_fields",
    "return_documents",
];
const COHERE_V2_PARAMS: [&str; 6] = [
    "query",
    "documents",
    "top_n",
    "max_tokens_per_doc",
    "rank_fields",
    "return_documents",
];
const JINA_PARAMS: [&str; 4] = ["query", "top_n", "documents", "return_documents"];
const VOYAGE_PARAMS: [&str; 4] = ["query", "documents", "top_n", "return_documents"];

// the contract's own methods, which preparation reaches through the trait rather than by name
#[rstest]
#[case(
    RerankConfig::Cohere,
    Some("https://co.test"),
    "https://co.test/v1/rerank",
    &COHERE_V1_PARAMS
)]
#[case(
    RerankConfig::CohereV2,
    Some("https://co.test"),
    "https://co.test/v2/rerank",
    &COHERE_V2_PARAMS
)]
#[case(
    RerankConfig::Infinity,
    Some("https://inf.test"),
    "https://inf.test/rerank",
    &COHERE_V1_PARAMS
)]
#[case(
    RerankConfig::JinaAi,
    Some("https://jina.test"),
    "https://jina.test/v1/rerank",
    &JINA_PARAMS
)]
#[case(
    RerankConfig::Voyage,
    Some("https://voyage.test"),
    "https://voyage.test/v1/rerank",
    &VOYAGE_PARAMS
)]
fn the_rerank_config_names_its_params_and_url(
    #[case] config: RerankConfig,
    #[case] base: Option<&str>,
    #[case] expected_url: &str,
    #[case] expected_params: &[&str],
) {
    let cohere = cohere_rerank_settings::Settings::default();
    let host = jina_infinity_rerank::Settings::default();
    let environment = BTreeMap::new();
    let family = FamilyRerankConfig {
        config,
        cohere: &cohere,
        host: &host,
        environment: &environment,
    };
    assert_eq!(
        family.get_complete_url(base, "rerank-v3.5", None),
        expected_url
    );
    assert_eq!(
        family.supported_rerank_params("rerank-v3.5"),
        expected_params
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>()
    );
}

#[test]
fn the_rerank_config_rewrites_the_raw_response() {
    let cohere = cohere_rerank_settings::Settings::default();
    let host = jina_infinity_rerank::Settings::default();
    let environment = BTreeMap::new();
    let family = FamilyRerankConfig {
        config: RerankConfig::Cohere,
        cohere: &cohere,
        host: &host,
        environment: &environment,
    };
    let raw = RerankProviderResponse {
        status_code: 200,
        headers: RerankHeaders::new(),
        body: RERANK_REPLY.as_bytes().to_vec(),
    };
    let mut logging = RerankLogging::default();
    let params = Params::new();
    let answer = family
        .transform_rerank_response(
            RerankResponse::default(),
            ResponseTransform {
                model: "rerank-v3.5",
                raw_response: &raw,
                logging: &mut logging,
                api_key: None,
                request_data: &params,
                optional_params: &params,
                litellm_params: &params,
            },
        )
        .unwrap();
    assert_eq!(answer.id.as_deref(), Some("r-1"));
    assert_eq!(answer.results.unwrap()[0].index, 0);
}

#[test]
fn an_unparsable_rerank_base_names_its_family() {
    let mut v1 = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    v1.setup.api_base = Some(String::from("not a url"));
    assert!(matches!(
        v1.prepare(),
        Err(FamilyError::Url {
            family: "cohere",
            ..
        })
    ));

    let mut v2 = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    v2.setup.api_base = Some(String::from("not a url"));
    v2.options.max_tokens_per_doc = Some(1);
    assert!(matches!(
        v2.prepare(),
        Err(FamilyError::Url {
            family: "cohere",
            ..
        })
    ));

    let mut voyage = RerankFixture::new(ProviderId::Voyage, "rerank-2");
    voyage.setup.api_base = Some(String::from("not a url"));
    assert!(matches!(
        voyage.prepare(),
        Err(FamilyError::Url {
            family: "voyage",
            ..
        })
    ));
}
