#![cfg(test)]

use rstest::rstest;

use crate::image_models::params::{Field, Params};
use crate::nova_embeddings::data_url::parse_nova_data_url;
use crate::nova_embeddings::error::NovaEmbeddingsError;
use crate::nova_embeddings::request::{
    SCHEMA_VERSION, SUPPORTED_OPENAI_PARAMS, map_openai_params, transform_request,
};
use crate::nova_embeddings::response::{
    NovaEmbeddingResponse, NovaPromptTokensDetails, transform_invoke_response, transform_response,
};

fn text(value: &str) -> Field {
    Field::Text(value.to_owned())
}

fn int(value: i64) -> Field {
    Field::Int(value)
}

fn float_field(value: f64) -> Field {
    Field::Float(value)
}

fn list_field(items: &[Field]) -> Field {
    Field::List(items.to_vec())
}

fn params(pairs: &[(&str, Field)]) -> Params {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn mapping_field(pairs: &[(&str, Field)]) -> Field {
    Field::Mapping(params(pairs))
}

fn single_params(request: &Params) -> &Params {
    match request.get("singleEmbeddingParams") {
        Some(Field::Mapping(map)) => map,
        _ => panic!("expected singleEmbeddingParams mapping"),
    }
}

fn segmented_params(request: &Params) -> &Params {
    match request.get("segmentedEmbeddingParams") {
        Some(Field::Mapping(map)) => map,
        _ => panic!("expected segmentedEmbeddingParams mapping"),
    }
}

fn embedding_len(field: &Field) -> usize {
    match field {
        Field::List(items) => items.len(),
        Field::Text(value) => value.chars().count(),
        Field::Null | Field::Bool(_) | Field::Int(_) | Field::Float(_) | Field::Mapping(_) => {
            panic!("expected list or text embedding")
        }
    }
}

fn assert_text_entry(request: &Params, value: &str) {
    match single_params(request).get("text") {
        Some(Field::Mapping(entry)) => {
            assert_eq!(entry.get("value"), Some(&text(value)));
            assert_eq!(entry.get("truncationMode"), Some(&text("END")));
        }
        _ => panic!("expected text mapping"),
    }
}

#[rstest]
#[case::nova_default_purpose_dimensions(nova_default_purpose_dimensions)]
#[case::nova_text_image_audio_video(nova_text_image_audio_video)]
#[case::nova_data_url_jpg(nova_data_url_jpg)]
#[case::nova_explicit_media_input(nova_explicit_media_input)]
#[case::nova_async_output_s3_required(nova_async_output_s3_required)]
#[case::nova_result_index_token_estimate(nova_result_index_token_estimate)]
#[case::nova_invocation_arn(nova_invocation_arn)]
fn nova_embeddings_cases(#[case] check: fn()) {
    check();
}

fn nova_default_purpose_dimensions() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, &["dimensions"]);

    let non_default = params(&[("dimensions", int(512)), ("other", text("ignored"))]);
    let mapped = map_openai_params(&non_default, Params::new());
    assert_eq!(mapped, params(&[("embedding_dimension", int(512))]));

    let inference = params(&[
        ("dimensions", int(256)),
        ("embedding_dimension", int(999)),
        ("output_s3_uri", text("s3://unused")),
        ("custom", text("keep")),
    ]);
    let default_request = transform_request("hello world", &inference, false, None, None).unwrap();
    assert_eq!(
        default_request.get("schemaVersion"),
        Some(&Field::Text(SCHEMA_VERSION.to_owned()))
    );
    assert_eq!(
        default_request.get("taskType"),
        Some(&text("SINGLE_EMBEDDING"))
    );
    let single = single_params(&default_request);
    assert_eq!(single.get("embeddingDimension"), Some(&int(256)));
    assert_eq!(single.get("embedding_dimension"), Some(&int(999)));
    assert_eq!(single.get("custom"), Some(&text("keep")));
    assert!(!single.contains_key("output_s3_uri"));
    assert_eq!(single.get("embeddingPurpose"), Some(&text("GENERIC_INDEX")));

    let explicit = params(&[
        ("embeddingPurpose", Field::Null),
        ("embeddingDimension", int(0)),
    ]);
    let explicit_request = transform_request("hello", &explicit, true, None, None).unwrap();
    let segmented = segmented_params(&explicit_request);
    assert_eq!(segmented.get("embeddingPurpose"), Some(&Field::Null));
    assert_eq!(segmented.get("embeddingDimension"), Some(&int(0)));
    assert_eq!(
        explicit_request.get("taskType"),
        Some(&text("SEGMENTED_EMBEDDING"))
    );

    assert_dimension_mapping_defaults();
}

fn assert_dimension_mapping_defaults() {
    let dimension_only = params(&[("embedding_dimension", int(128))]);
    let dimension_only_request =
        transform_request("hello", &dimension_only, false, None, None).unwrap();
    let dimension_only_params = single_params(&dimension_only_request);
    assert_eq!(
        dimension_only_params.get("embeddingDimension"),
        Some(&int(128))
    );
    assert!(!dimension_only_params.contains_key("embedding_dimension"));

    let bare_request = transform_request("hello", &Params::new(), false, None, None).unwrap();
    assert_eq!(
        single_params(&bare_request).get("embeddingDimension"),
        Some(&int(3072))
    );
}

fn nova_text_image_audio_video() {
    assert_plain_and_https_text();
    assert_s3_text();
    assert_image_video_audio_media();
    assert_unknown_media_falls_back_to_text();
}

fn assert_plain_and_https_text() {
    let inference = Params::new();
    let plain = transform_request("plain text", &inference, false, None, None).unwrap();
    assert_text_entry(&plain, "plain text");

    let https = transform_request("https://example.com/x", &inference, false, None, None).unwrap();
    assert_text_entry(&https, "https://example.com/x");
}

fn assert_s3_text() {
    let inference = Params::new();
    let s3 = transform_request("s3://bucket/key", &inference, false, None, None).unwrap();
    match single_params(&s3).get("text") {
        Some(Field::Mapping(entry)) => {
            assert_eq!(entry.get("truncationMode"), Some(&text("END")));
            match entry.get("source") {
                Some(Field::Mapping(source)) => match source.get("s3Location") {
                    Some(Field::Mapping(location)) => {
                        assert_eq!(location.get("uri"), Some(&text("s3://bucket/key")));
                    }
                    _ => panic!("expected s3Location mapping"),
                },
                _ => panic!("expected source mapping"),
            }
        }
        _ => panic!("expected text mapping"),
    }
}

fn assert_image_video_audio_media() {
    let inference = Params::new();

    let image =
        transform_request("data:image/PNG;base64,ABCD", &inference, false, None, None).unwrap();
    match single_params(&image).get("image") {
        Some(Field::Mapping(entry)) => {
            assert_eq!(entry.get("format"), Some(&text("png")));
            match entry.get("source") {
                Some(Field::Mapping(source)) => {
                    assert_eq!(source.get("bytes"), Some(&text("ABCD")));
                }
                _ => panic!("expected source mapping"),
            }
        }
        _ => panic!("expected image mapping"),
    }

    let video =
        transform_request("data:video/MP4,RAWBYTES", &inference, false, None, None).unwrap();
    match single_params(&video).get("video") {
        Some(Field::Mapping(entry)) => assert_eq!(entry.get("format"), Some(&text("mp4"))),
        _ => panic!("expected video mapping"),
    }

    let audio =
        transform_request("data:audio/MPEG,RAWBYTES", &inference, false, None, None).unwrap();
    match single_params(&audio).get("audio") {
        Some(Field::Mapping(entry)) => assert_eq!(entry.get("format"), Some(&text("mpeg"))),
        _ => panic!("expected audio mapping"),
    }
}

fn assert_unknown_media_falls_back_to_text() {
    let inference = Params::new();
    let whole = "data:application/octet-stream;base64,ZZZ";
    let unknown = transform_request(whole, &inference, false, None, None).unwrap();
    assert_text_entry(&unknown, whole);
}

fn nova_data_url_jpg() {
    assert_parse_nova_data_url_variants();
    assert_transform_request_jpg_to_jpeg();
}

fn assert_parse_nova_data_url_variants() {
    let parsed = parse_nova_data_url("data:image/jpg;base64,/9j/4AAQ,extra,commas").unwrap();
    assert_eq!(parsed.media_type, "image/jpg");
    assert_eq!(parsed.data, "/9j/4AAQ,extra,commas");

    let no_semicolon = parse_nova_data_url("data:text/plain,hello").unwrap();
    assert_eq!(no_semicolon.media_type, "text/plain");
    assert_eq!(no_semicolon.data, "hello");

    let empty_payload = parse_nova_data_url("data:,").unwrap();
    assert_eq!(empty_payload.media_type, "");
    assert_eq!(empty_payload.data, "");

    assert!(matches!(
        parse_nova_data_url("image/jpeg;base64,ZZZ"),
        Err(NovaEmbeddingsError::InvalidDataUrl { .. })
    ));
    assert!(matches!(
        parse_nova_data_url("data:image/jpeg;base64"),
        Err(NovaEmbeddingsError::MissingComma { .. })
    ));
    assert!(matches!(
        parse_nova_data_url("DATA:image/jpeg,ZZZ"),
        Err(NovaEmbeddingsError::InvalidDataUrl { .. })
    ));
}

fn assert_transform_request_jpg_to_jpeg() {
    let request = transform_request(
        "data:image/jpg;base64,/9j/4AAQ==",
        &Params::new(),
        false,
        None,
        None,
    )
    .unwrap();
    match single_params(&request).get("image") {
        Some(Field::Mapping(entry)) => {
            assert_eq!(entry.get("format"), Some(&text("jpeg")));
            match entry.get("source") {
                Some(Field::Mapping(source)) => {
                    assert_eq!(source.get("bytes"), Some(&text("/9j/4AAQ==")));
                }
                _ => panic!("expected source mapping"),
            }
        }
        _ => panic!("expected image mapping"),
    }
}

fn nova_explicit_media_input() {
    let mut inference = Params::new();
    inference.insert("text".to_owned(), Field::Null);
    inference.insert(
        "image".to_owned(),
        mapping_field(&[
            ("format", text("png")),
            ("source", mapping_field(&[("bytes", text("xyz"))])),
        ]),
    );

    let malformed_input = "data:badformat_no_comma";
    let request = transform_request(malformed_input, &inference, false, None, None).unwrap();
    let embedding_params = single_params(&request);
    assert_eq!(embedding_params.get("text"), Some(&Field::Null));
    assert!(embedding_params.contains_key("image"));
}

fn nova_async_output_s3_required() {
    let request = transform_request("hello", &Params::new(), true, None, None).unwrap();
    assert_eq!(request.get("taskType"), Some(&text("SEGMENTED_EMBEDDING")));
    assert!(!request.contains_key("modelId"));

    assert_missing_or_blank_output_s3_uri();
    assert_wrapped_async_invoke_request();
}

fn assert_missing_or_blank_output_s3_uri() {
    let mut leaking = Params::new();
    leaking.insert("output_s3_uri".to_owned(), text("s3://wrong"));
    let missing_uri_error =
        transform_request("hello", &leaking, true, Some("model-x"), None).unwrap_err();
    assert!(matches!(
        missing_uri_error,
        NovaEmbeddingsError::OutputS3UriRequired
    ));

    let blank_uri_error =
        transform_request("hello", &Params::new(), true, Some("model-x"), Some("   ")).unwrap_err();
    assert!(matches!(
        blank_uri_error,
        NovaEmbeddingsError::OutputS3UriRequired
    ));
}

fn assert_wrapped_async_invoke_request() {
    let wrapped = transform_request(
        "hello",
        &Params::new(),
        true,
        Some("async_invoke/async_invoke/model%2Fx"),
        Some(" s3://bucket/output/ "),
    )
    .unwrap();
    assert_eq!(wrapped.get("modelId"), Some(&text("model/x")));
    match wrapped.get("modelInput") {
        Some(Field::Mapping(model_input)) => {
            assert_eq!(
                model_input.get("taskType"),
                Some(&text("SEGMENTED_EMBEDDING"))
            );
        }
        _ => panic!("expected modelInput mapping"),
    }
    match wrapped.get("outputDataConfig") {
        Some(Field::Mapping(output_config)) => match output_config.get("s3OutputDataConfig") {
            Some(Field::Mapping(s3_config)) => {
                assert_eq!(s3_config.get("s3Uri"), Some(&text(" s3://bucket/output/ ")));
            }
            _ => panic!("expected s3OutputDataConfig mapping"),
        },
        _ => panic!("expected outputDataConfig mapping"),
    }
}

fn build_response_with_embeddings_list() -> Params {
    let entry_with_truncation = mapping_field(&[
        (
            "embedding",
            list_field(&[float_field(0.1), float_field(0.2), float_field(0.3)]),
        ),
        ("truncatedCharLength", int(40)),
    ]);
    let entry_without_truncation = mapping_field(&[(
        "embedding",
        list_field(&[
            float_field(0.4),
            float_field(0.5),
            float_field(0.6),
            float_field(0.7),
        ]),
    )]);
    let entry_missing_embedding = mapping_field(&[("embeddingType", text("TEXT"))]);

    params(&[(
        "embeddings",
        Field::List(vec![
            entry_with_truncation,
            entry_without_truncation,
            entry_missing_embedding,
        ]),
    )])
}

fn build_nova_response_list() -> [Params; 4] {
    let response_with_embeddings = build_response_with_embeddings_list();

    let response_empty_embeddings_with_fallback = params(&[
        ("embeddings", Field::List(Vec::new())),
        (
            "embedding",
            list_field(&[float_field(1.0), float_field(2.0)]),
        ),
    ]);

    let response_direct_embedding = params(&[(
        "embedding",
        list_field(&[
            float_field(9.0),
            float_field(8.0),
            float_field(7.0),
            float_field(6.0),
            float_field(5.0),
        ]),
    )]);

    [
        response_with_embeddings,
        response_empty_embeddings_with_fallback,
        response_direct_embedding,
        Params::new(),
    ]
}

fn build_nova_batch_data() -> [Params; 3] {
    [
        params(&[(
            "singleEmbeddingParams",
            mapping_field(&[("image", text("present"))]),
        )]),
        params(&[(
            "segmentedEmbeddingParams",
            mapping_field(&[("image", text("present"))]),
        )]),
        params(&[("singleEmbeddingParams", mapping_field(&[]))]),
    ]
}

fn assert_nova_result_shape(result: &NovaEmbeddingResponse) {
    assert_eq!(result.model, "nova-model");
    assert_eq!(result.data.len(), 3);
    assert_eq!(embedding_len(&result.data[0].embedding), 3);
    assert_eq!(embedding_len(&result.data[1].embedding), 4);
    assert_eq!(embedding_len(&result.data[2].embedding), 5);

    assert_eq!(result.usage.prompt_tokens, 12);
    assert_eq!(result.usage.total_tokens, 12);
    assert_eq!(
        result.usage.prompt_tokens_details,
        Some(NovaPromptTokensDetails { image_count: 2 })
    );
}

fn nova_result_index_token_estimate() {
    let response_list = build_nova_response_list();
    let batch_data = build_nova_batch_data();

    let result = transform_response(&response_list, "nova-model", Some(&batch_data)).unwrap();
    assert_nova_result_shape(&result);

    // embedding is list | str; a string's character count feeds the token estimate too.
    let text_embedding = params(&[("embedding", text("12345678"))]);
    let text_result = transform_response(&[text_embedding], "nova-model", None).unwrap();
    assert_eq!(text_result.data[0].embedding, text("12345678"));
    assert_eq!(text_result.usage.total_tokens, 2);

    let malformed = params(&[("embedding", int(42))]);
    let error = transform_response(&[malformed], "nova-model", None).unwrap_err();
    assert!(matches!(
        error,
        NovaEmbeddingsError::MalformedField { field: "embedding" }
    ));

    let no_images = transform_response(&[Params::new()], "nova-model", None).unwrap();
    assert_eq!(no_images.usage.prompt_tokens_details, None);

    assert_list_entry_type_checked_before_truncation();
    assert_non_object_list_entries();
}

// LiteLLM builds Embedding(embedding=item["embedding"], ...) before it reads truncatedCharLength.
fn assert_list_entry_type_checked_before_truncation() {
    let null_embedding = params(&[(
        "embeddings",
        Field::List(vec![mapping_field(&[
            ("embedding", Field::Null),
            ("truncatedCharLength", int(8)),
        ])]),
    )]);
    let error = transform_response(&[null_embedding], "nova-model", None).unwrap_err();
    assert!(matches!(
        error,
        NovaEmbeddingsError::MalformedField { field: "embedding" }
    ));
}

fn assert_non_object_list_entries() {
    for entry in [Field::Null, Field::Bool(true), int(1), float_field(1.5)] {
        let response = params(&[("embeddings", Field::List(vec![entry]))]);
        let error = transform_response(&[response], "nova-model", None).unwrap_err();
        assert!(matches!(
            error,
            NovaEmbeddingsError::MalformedField {
                field: "embeddings"
            }
        ));
    }

    let skipped = params(&[(
        "embeddings",
        Field::List(vec![list_field(&[]), text("not a mapping")]),
    )]);
    let skipped_result = transform_response(&[skipped], "nova-model", None).unwrap();
    assert_eq!(skipped_result.data.len(), 0);

    assert_text_and_list_entries_with_embedding_key();
}

// A str holding "embedding" as a substring, or a list holding it as an element, raises too: item["embedding"] fails on both the same way it does on null, a number or a bool.
fn assert_text_and_list_entries_with_embedding_key() {
    let text_entry = params(&[("embeddings", Field::List(vec![text("an embedding")]))]);
    let text_error = transform_response(&[text_entry], "nova-model", None).unwrap_err();
    assert!(matches!(
        text_error,
        NovaEmbeddingsError::MalformedField {
            field: "embeddings"
        }
    ));

    let list_entry = params(&[(
        "embeddings",
        Field::List(vec![list_field(&[text("embedding")])]),
    )]);
    let list_error = transform_response(&[list_entry], "nova-model", None).unwrap_err();
    assert!(matches!(
        list_error,
        NovaEmbeddingsError::MalformedField {
            field: "embeddings"
        }
    ));
}

fn nova_invocation_arn() {
    let response = params(&[(
        "invocationArn",
        text("arn:aws:bedrock:us-east-1:123:async-invoke/abc"),
    )]);
    let present_result = transform_invoke_response(&response, "nova-model");
    assert_eq!(present_result.model, "nova-model");
    assert_eq!(present_result.data.len(), 1);
    assert_eq!(present_result.data[0].embedding, Field::List(Vec::new()));
    assert_eq!(present_result.usage.prompt_tokens, 0);
    assert_eq!(present_result.usage.total_tokens, 0);
    assert_eq!(
        present_result.invocation_arn,
        Some(text("arn:aws:bedrock:us-east-1:123:async-invoke/abc"))
    );

    let missing = Params::new();
    let missing_result = transform_invoke_response(&missing, "nova-model");
    assert_eq!(missing_result.invocation_arn, Some(text("")));

    let explicit_none = params(&[("invocationArn", Field::Null)]);
    let null_result = transform_invoke_response(&explicit_none, "nova-model");
    assert_eq!(null_result.invocation_arn, Some(Field::Null));
}
