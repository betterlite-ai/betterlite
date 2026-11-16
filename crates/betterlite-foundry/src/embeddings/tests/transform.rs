use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json as HeaderJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::base64_string::is_base64_encoded;
use indexmap::IndexMap;

use crate::embeddings::call::projected_headers;
use crate::embeddings::cohere::map_azure_model_group;
use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::request::{FoundryEmbeddingOptions, transform_request};

use super::support::{
    Answer, Factory, IMAGE_ONE, IMAGE_URI, MODEL, TEXT_EMPTY, TEXT_ONE, Text, header_map, indices,
    key, object, pairs, request, run, strings,
};

const IMAGE_TWO: &str = r#"{"object":"list","data":[{"object":"embedding","index":5,"embedding":[0.5]},{"object":"embedding","index":6,"embedding":[0.6]}],"model":"image-model","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
const TEXT_TWO: &str = r#"{"object":"list","data":[{"object":"embedding","index":7,"embedding":[0.7]},{"object":"embedding","index":9,"embedding":[0.9]}],"model":"text-model","usage":{"prompt_tokens":3,"total_tokens":3}}"#;

pub(super) fn cohere_model_group() {
    mapped_groups();
    text_group_header();
    image_group_header();
    raw_group_headers();
}

fn mapped_groups() {
    assert_eq!(
        map_azure_model_group("offer-cohere-embed-multili-paygo"),
        "Cohere-embed-v3-multilingual"
    );
    assert_eq!(
        map_azure_model_group("offer-cohere-embed-english-paygo"),
        "Cohere-embed-v3-english"
    );
    assert_eq!(
        map_azure_model_group("offer-cohere-embed-multilingual"),
        "offer-cohere-embed-multilingual"
    );
    assert_eq!(map_azure_model_group(""), "");
}

fn text_group_header() {
    let input = strings(&["hello"]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let multilingual = [(
        "llm_provider-azureml-model-group",
        "offer-cohere-embed-multili-paygo",
    )];
    let response = run(
        &call,
        None,
        &factory,
        &Text::headed(&multilingual, TEXT_ONE),
    )
    .unwrap();
    assert_eq!(
        response.model.as_deref(),
        Some("Cohere-embed-v3-multilingual")
    );
    let english = [(
        "llm_provider-azureml-model-group",
        "offer-cohere-embed-english-paygo",
    )];
    let response = run(&call, None, &factory, &Text::headed(&english, TEXT_ONE)).unwrap();
    assert_eq!(response.model.as_deref(), Some("Cohere-embed-v3-english"));
    let custom = [("llm_provider-azureml-model-group", "custom-group")];
    let response = run(&call, None, &factory, &Text::headed(&custom, TEXT_ONE)).unwrap();
    assert_eq!(response.model.as_deref(), Some("custom-group"));
    let empty = [("llm_provider-azureml-model-group", "")];
    let response = run(&call, None, &factory, &Text::headed(&empty, TEXT_ONE)).unwrap();
    assert_eq!(response.model.as_deref(), Some("text-model"));
}

fn image_group_header() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let headers = [(
        "llm_provider-azureml-model-group",
        "offer-cohere-embed-english-paygo",
    )];
    let factory = Factory::new(Answer::headed(&headers, IMAGE_ONE));
    let response = run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(response.model.as_deref(), Some("Cohere-embed-v3-english"));
}

// the reply sends the provider's own header name; only the projection adds llm_provider-
fn raw_group_headers() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let english = [("azureml-model-group", "offer-cohere-embed-english-paygo")];
    let factory = Factory::new(Answer::headed(&english, IMAGE_ONE));
    let response = run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(response.model.as_deref(), Some("Cohere-embed-v3-english"));
    let input = strings(&["hello"]);
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let multilingual = [("azureml-model-group", "offer-cohere-embed-multili-paygo")];
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let response = run(
        &call,
        None,
        &factory,
        &Text::headed(&multilingual, TEXT_ONE),
    )
    .unwrap();
    assert_eq!(
        response.model.as_deref(),
        Some("Cohere-embed-v3-multilingual")
    );
}

pub(super) fn text_input_type_truncate() {
    classification();
    options_copied();
    filtered_only_tests_emptiness();
    all_text_and_all_images();
}

fn classification() {
    assert!(is_base64_encoded(IMAGE_URI));
    assert!(is_base64_encoded("data:,"));
    assert!(is_base64_encoded("data:image/png;base64,AAAA,BBBB"));
    assert!(!is_base64_encoded("Dog"));
    assert!(!is_base64_encoded(""));
    assert!(!is_base64_encoded("DATA:image/png;base64,AAAA"));
    assert!(!is_base64_encoded("data:image/png;base64,!!!!"));
    assert!(!is_base64_encoded("data:image/png;base64,AAA"));
    assert!(!is_base64_encoded("data:image/png;base64,AB=="));
    assert!(!is_base64_encoded("data"));
}

fn options_copied() {
    let input = strings(&["hello", IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    call.options = FoundryEmbeddingOptions {
        input_type: Some("search_document".to_owned()),
        truncate: Some("END".to_owned()),
        passthrough: IndexMap::from([("custom".to_owned(), Json::Text("value".to_owned()))]),
    };
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_ONE);
    run(&call, None, &factory, &text).unwrap();
    let requests = factory.requests();
    assert_eq!(requests.len(), 1);
    let body: Object = serde_json::from_slice(&requests[0].body).unwrap();
    let expected = object(&format!(
        r#"{{"input":[{{"image":"{IMAGE_URI}"}}],"input_type":"search_document","truncate":"END","custom":"value"}}"#
    ));
    assert_eq!(body, expected);
    let calls = text.calls();
    assert_eq!(
        calls[0].options.input_type.as_deref(),
        Some("search_document")
    );
    assert_eq!(calls[0].options.truncate.as_deref(), Some("END"));
    assert_eq!(
        calls[0].options.passthrough.get("custom"),
        Some(&Json::Text("value".to_owned()))
    );
}

fn filtered_only_tests_emptiness() {
    let input = strings(&["hello", IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_ONE);
    run(&call, None, &factory, &text).unwrap();
    assert_eq!(text.calls()[0].input, input);
    let plan = transform_request(&input, &FoundryEmbeddingOptions::default());
    assert_eq!(plan.text_input, vec!["hello"]);
    assert_eq!(plan.image_indices, vec![1]);
    assert_eq!(plan.image.input.len(), 1);
    assert_eq!(plan.image.input[0].image, IMAGE_URI);
}

fn all_text_and_all_images() {
    let texts = strings(&["one", "two"]);
    let plan = transform_request(&texts, &FoundryEmbeddingOptions::default());
    assert!(plan.image.input.is_empty());
    assert!(plan.image_indices.is_empty());
    assert_eq!(plan.text_input, vec!["one", "two"]);
    let images = strings(&[IMAGE_URI, IMAGE_URI]);
    let plan = transform_request(&images, &FoundryEmbeddingOptions::default());
    assert_eq!(plan.image_indices, vec![0, 1]);
    assert!(plan.text_input.is_empty());
    assert_eq!(plan.image.input[1].image, IMAGE_URI);
}

pub(super) fn response_model_index_usage() {
    combined_order();
    available_lists();
    usage_from_headers();
    usage_from_raw_header();
    usage_from_last_conversion();
    projection_keeps_llm_provider_names();
    short_lists();
}

fn combined_order() {
    let input = strings(&["hello", IMAGE_URI, "world"]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_TWO));
    let text = Text::replying(TEXT_TWO);
    let response = run(&call, None, &factory, &text).unwrap();
    assert_eq!(indices(&response.data), vec![7, 5, 9]);
    assert_eq!(response.model.as_deref(), Some("text-model"));
    assert_eq!(response.usage.prompt_tokens, 3);
}

fn available_lists() {
    let images = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &images);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_TWO));
    let response = run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(indices(&response.data), vec![5, 6]);
    assert_eq!(response.model.as_deref(), Some("image-model"));
    let texts = strings(&["hello"]);
    let mut call = request(MODEL, &texts);
    call.api_key = Some(&secret);
    let response = run(&call, None, &factory, &Text::replying(TEXT_TWO)).unwrap();
    assert_eq!(indices(&response.data), vec![7, 9]);
    assert_eq!(response.model.as_deref(), Some("text-model"));
    let empty: Vec<String> = Vec::new();
    let mut call = request(MODEL, &empty);
    call.api_key = Some(&secret);
    let fresh = Factory::new(Answer::json(IMAGE_TWO));
    let text = Text::replying(TEXT_TWO);
    let response = run(&call, None, &fresh, &text).unwrap();
    assert!(response.data.is_empty());
    assert!(fresh.requests().is_empty());
    assert!(fresh.sync_timeouts().is_empty());
    assert!(text.calls().is_empty());
}

fn usage_from_headers() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let tokens = [("llm_provider-num_tokens", "9")];
    let factory = Factory::new(Answer::headed(&tokens, IMAGE_ONE));
    let response = run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(response.usage.prompt_tokens, 9);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.total_tokens, 2);
    let bare = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1]}],"model":"image-model"}"#;
    let factory = Factory::new(Answer::headed(&tokens, bare));
    let response = run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(response.usage.prompt_tokens, 9);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
}

// the reply sends num_tokens; only the projection turns it into the llm_provider-name the transform reads
fn usage_from_raw_header() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let tokens = [("num_tokens", "9")];
    let factory = Factory::new(Answer::headed(&tokens, IMAGE_ONE));
    let response = run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(response.usage.prompt_tokens, 9);
    assert_eq!(response.usage.total_tokens, 2);
}

fn projection_keeps_llm_provider_names() {
    let headers = header_map(&pairs(&[
        ("x-ratelimit-reset-requests", "6m"),
        ("x-ratelimit-limit-requests", "100"),
        ("x-llm_provider-x", "y"),
        ("x-provider-header", "z"),
    ]));
    let expected = AdditionalHeaders::from([
        (
            "llm_provider-x-ratelimit-reset-requests".to_owned(),
            HeaderJson::Text("6m".to_owned()),
        ),
        (
            "llm_provider-x-ratelimit-limit-requests".to_owned(),
            HeaderJson::Text("100".to_owned()),
        ),
        (
            "x-llm_provider-x".to_owned(),
            HeaderJson::Text("y".to_owned()),
        ),
        (
            "llm_provider-x-provider-header".to_owned(),
            HeaderJson::Text("z".to_owned()),
        ),
        (
            "x-ratelimit-limit-requests".to_owned(),
            HeaderJson::Text("100".to_owned()),
        ),
    ]);
    assert_eq!(projected_headers(&headers), expected);
}

fn usage_from_last_conversion() {
    let input = strings(&["hello", IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let image_headers = [
        ("llm_provider-num_tokens", "9"),
        (
            "llm_provider-azureml-model-group",
            "offer-cohere-embed-english-paygo",
        ),
    ];
    let image_body = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1]}],"model":"image-model","usage":{"prompt_tokens":4,"completion_tokens":1,"total_tokens":5}}"#;
    let text_body = r#"{"object":"list","data":[{"object":"embedding","index":1,"embedding":[0.2]}],"model":"text-model"}"#;
    let factory = Factory::new(Answer::headed(&image_headers, image_body));
    let text = Text::headed(&[("llm_provider-num_tokens", " 42 ")], text_body);
    let response = run(&call, None, &factory, &text).unwrap();
    assert_eq!(indices(&response.data), vec![1, 0]);
    assert_eq!(response.model.as_deref(), Some("text-model"));
    assert_eq!(response.usage.prompt_tokens, 42);
    assert_eq!(response.usage.completion_tokens, 1);
    assert_eq!(response.usage.total_tokens, 5);
}

fn short_lists() {
    let secret = key("case-key");
    let mixed = strings(&[IMAGE_URI, "hello"]);
    let mut call = request(MODEL, &mixed);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let empty_text = r#"{"object":"list","data":[],"model":"text-model"}"#;
    let error = run(&call, None, &factory, &Text::replying(empty_text)).unwrap_err();
    assert!(matches!(
        error,
        FoundryEmbeddingError::TooFewEntries {
            source: "text embeddings"
        }
    ));
    let empty_image = r#"{"object":"list","data":[],"model":"image-model"}"#;
    let factory = Factory::new(Answer::json(empty_image));
    let error = run(&call, None, &factory, &Text::replying(TEXT_ONE)).unwrap_err();
    assert!(matches!(
        error,
        FoundryEmbeddingError::TooFewEntries {
            source: "image embeddings"
        }
    ));
}
