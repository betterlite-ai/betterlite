use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::huggingface_embedding::response::OrdinaryEncoder as _;
use betterlite_openai::huggingface_rerank::TextTokenCounter as _;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret, SecretString};

use super::super::rerank::huggingface_rerank_config;
use super::super::tokens::Cl100k;
use super::support::{empty_globals, environment};
use crate::api::rerank_contracts::{
    BaseRerankConfig as _, EnvironmentRequest, MapCohereRerankParams, Params, RerankLogging,
    RerankProviderResponse, ResponseTransform,
};

#[test]
fn the_rerank_config_takes_its_key_and_base_from_the_chains() {
    let hf_global = SecretString::from("hf-global");
    let mut globals = empty_globals();
    globals.huggingface_key = Some(&hf_global);
    let env = environment(&[
        ("HUGGINGFACE_API_KEY", "hf-api"),
        ("HF_API_BASE", "http://hf"),
    ]);
    let call_key = SecretString::from("call");
    let explicit =
        huggingface_rerank_config(Some(&call_key), Some("http://call"), &globals, &env).unwrap();
    assert_eq!(
        explicit.key.as_ref().map(ExposeSecret::expose_secret),
        Some("call")
    );
    assert_eq!(explicit.api_base.as_deref(), Some("http://call"));
    let inherited = huggingface_rerank_config(None, None, &globals, &env).unwrap();
    assert_eq!(
        inherited.key.as_ref().map(ExposeSecret::expose_secret),
        Some("hf-global")
    );
    assert_eq!(inherited.api_base.as_deref(), Some("http://hf"));
    assert_eq!(
        inherited.get_complete_url(None, "m", None),
        "http://hf/rerank"
    );
    let request = EnvironmentRequest {
        headers: &Headers::new(),
        model: "m",
        api_key: None,
        optional_params: None,
        litellm_params: None,
    };
    let headers = inherited.validate_environment(request).unwrap();
    assert_eq!(headers["Authorization"], "Bearer hf-global");
    assert!(
        inherited
            .supported_rerank_params("m")
            .contains(&String::from("query"))
    );
}

#[test]
fn the_rerank_config_maps_the_cohere_params_and_builds_the_body() {
    let config =
        huggingface_rerank_config(None, None, &empty_globals(), &environment(&[])).unwrap();
    let documents = Json::List(vec![Json::Text(String::from("d"))]);
    let mut cohere = Params::from([
        (String::from("query"), Json::Text(String::from("q"))),
        (String::from("documents"), documents.clone()),
    ]);
    let mapped = config
        .map_cohere_rerank_params(MapCohereRerankParams {
            non_default_params: &mut cohere,
            model: "m",
            drop_params: false,
            query: "q",
            documents: &[],
            custom_llm_provider: None,
            top_n: None,
            rank_fields: None,
            return_documents: None,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        })
        .unwrap();
    assert_eq!(mapped.get("texts"), Some(&documents));
    let body = config
        .transform_rerank_request("m", &mapped, &Headers::new(), None)
        .unwrap();
    let direction = Json::Text(String::from("Right"));
    assert_eq!(body.get("truncation_direction"), Some(&direction));
}

#[test]
fn the_rerank_config_reads_the_reply_with_the_module_counter() {
    let config =
        huggingface_rerank_config(None, None, &empty_globals(), &environment(&[])).unwrap();
    let raw = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: br#"[{"index":0,"score":0.5}]"#.to_vec(),
    };
    let request = Params::from([
        (String::from("query"), Json::Text(String::from("q"))),
        (
            String::from("texts"),
            Json::List(vec![Json::Text(String::from("d"))]),
        ),
    ]);
    let empty = Params::new();
    let mut logging = RerankLogging::default();
    let transform = ResponseTransform {
        model: "m",
        raw_response: &raw,
        logging: &mut logging,
        api_key: None,
        request_data: &request,
        optional_params: &empty,
        litellm_params: &empty,
    };
    let response = config
        .transform_rerank_response(RerankResponse::default(), transform)
        .unwrap();
    assert!(response.id.is_some());
    assert_eq!(response.results.map(|results| results.len()), Some(1));
}

#[test]
fn the_cl100k_counter_counts_text() {
    let counter = Cl100k::new().unwrap();
    assert_eq!(counter.count("m", "hello world").unwrap(), 2);
    assert_eq!(counter.encode_ordinary("hello world"), 2);
}
