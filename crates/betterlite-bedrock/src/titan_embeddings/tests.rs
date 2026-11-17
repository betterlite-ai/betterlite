#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "each helper states one LiteLLM behavior as a single scenario"
    )
)]

use rstest::rstest;
use serde_json::Number;

use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::titan_embeddings::config::{
    ClassAttribute, ClassAttributes, TitanV2Settings, get_config,
};
use crate::titan_embeddings::error::TitanEmbeddingsError;
use crate::titan_embeddings::response::{TitanEmbeddingResponse, TitanPromptTokensDetails};
use crate::titan_embeddings::{cohere, g1, multimodal, response, v2};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn int(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite number"))
}

fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

fn object(pairs: Vec<(&str, Json)>) -> Object {
    let mut map = Object::new();
    for (key, value) in pairs {
        map.insert(key.to_owned(), value);
    }
    map
}

fn json_object(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(object(pairs))
}

fn numbers(values: &[i64]) -> EmbeddingVector {
    EmbeddingVector::Floats(values.iter().map(|value| Number::from(*value)).collect())
}

fn floats(values: &[f64]) -> EmbeddingVector {
    EmbeddingVector::Floats(
        values
            .iter()
            .map(|value| Number::from_f64(*value).expect("a finite number"))
            .collect(),
    )
}

fn attributes(pairs: Vec<(&str, ClassAttribute)>) -> ClassAttributes {
    let mut map = ClassAttributes::new();
    for (name, attribute) in pairs {
        map.insert(name.to_owned(), attribute);
    }
    map
}

#[rstest]
#[case::titan_g1_request(titan_g1_request)]
#[case::titan_v2_dimensions_normalize(titan_v2_dimensions_normalize)]
#[case::titan_v2_binary_embedding(titan_v2_binary_embedding)]
#[case::titan_multimodal_image(titan_multimodal_image)]
#[case::cohere_v3_input_type(cohere_v3_input_type)]
#[case::token_count_response(token_count_response)]
fn titan_embeddings_cases(#[case] check: fn()) {
    check();
}

fn titan_g1_request() {
    assert!(g1::SUPPORTED_OPENAI_PARAMS.is_empty());
    // _transform_request builds the record from inputText alone, so every inference parameter is dropped
    assert_eq!(
        g1::transform_request("hello world"),
        object(vec![("inputText", text("hello world"))])
    );
    assert_eq!(
        g1::transform_request(""),
        object(vec![("inputText", text(""))])
    );
    assert_g1_construction_has_no_settings();
    assert_shared_config_filter();
}

// AmazonTitanG1Config.__init__ copies locals() minus self, and the constructor takes no argument, so the class keeps no setting
fn assert_g1_construction_has_no_settings() {
    let empty = ClassAttributes::new();
    assert_eq!(get_config(&empty), Object::new());
    // the real class body holds only methods and dunders, and both kinds leave get_config
    let class_body = attributes(vec![
        ("__module__", ClassAttribute::Value(text("litellm"))),
        ("__doc__", ClassAttribute::Value(text("docs"))),
        ("__init__", ClassAttribute::Function),
        ("get_config", ClassAttribute::ClassMethod),
        ("get_supported_openai_params", ClassAttribute::Function),
        ("map_openai_params", ClassAttribute::Function),
        ("_transform_request", ClassAttribute::Function),
        ("_transform_response", ClassAttribute::Function),
    ]);
    assert_eq!(get_config(&class_body), Object::new());
}

fn assert_shared_config_filter() {
    let probed = attributes(vec![
        ("__hidden__", ClassAttribute::Value(text("dunder"))),
        ("_retained", ClassAttribute::Value(text("kept"))),
        ("_count", ClassAttribute::Value(int(3))),
        ("_absent", ClassAttribute::Value(Json::Null)),
        ("_function", ClassAttribute::Function),
        ("_builtin", ClassAttribute::BuiltinFunction),
        ("_class_method", ClassAttribute::ClassMethod),
        ("_static_method", ClassAttribute::StaticMethod),
    ]);
    assert_eq!(
        get_config(&probed),
        object(vec![("_retained", text("kept")), ("_count", int(3))])
    );
}

fn titan_v2_dimensions_normalize() {
    assert_v2_settings_default_to_none();
    assert_v2_settings_never_reset();
    assert_v2_mapping();
    assert_v2_request();
}

fn assert_v2_settings_default_to_none() {
    let settings = TitanV2Settings::default();
    assert_eq!(
        settings,
        TitanV2Settings {
            normalize: None,
            dimensions: None
        }
    );
    assert_eq!(get_config(&settings.attributes()), Object::new());
}

fn assert_v2_settings_never_reset() {
    let mut settings = TitanV2Settings::default();
    settings.apply(Some(true), None);
    assert_eq!(
        get_config(&settings.attributes()),
        object(vec![("normalize", Json::Bool(true))])
    );
    // a later AmazonTitanV2Config(normalize=None) writes nothing, so the class attribute stays true
    settings.apply(None, None);
    assert_eq!(
        get_config(&settings.attributes()),
        object(vec![("normalize", Json::Bool(true))])
    );
    settings.apply(None, Some(512));
    assert_eq!(
        get_config(&settings.attributes()),
        object(vec![
            ("normalize", Json::Bool(true)),
            ("dimensions", int(512)),
        ])
    );
    settings.apply(Some(false), None);
    assert_eq!(
        get_config(&settings.attributes()),
        object(vec![
            ("normalize", Json::Bool(false)),
            ("dimensions", int(512)),
        ])
    );
    // AmazonTitanV2Config(dimensions=None) writes nothing either
    settings.apply(None, None);
    assert_eq!(
        get_config(&settings.attributes()),
        object(vec![
            ("normalize", Json::Bool(false)),
            ("dimensions", int(512)),
        ])
    );
}

fn assert_v2_mapping() {
    assert_eq!(
        v2::SUPPORTED_OPENAI_PARAMS,
        &["dimensions", "encoding_format"]
    );
    let dimensions = v2::map_openai_params(&object(vec![("dimensions", int(512))]), Object::new());
    assert_eq!(dimensions.get("dimensions"), Some(&int(512)));
    let mapped_float = v2::map_openai_params(
        &object(vec![("encoding_format", text("float"))]),
        Object::new(),
    );
    assert_eq!(
        mapped_float.get("embeddingTypes"),
        Some(&list(vec![text("float")]))
    );
    let base64 = v2::map_openai_params(
        &object(vec![("encoding_format", text("base64"))]),
        Object::new(),
    );
    assert_eq!(
        base64.get("embeddingTypes"),
        Some(&list(vec![text("binary")]))
    );
    // every other encoding, an explicit null included, asks for float
    let int8 = v2::map_openai_params(
        &object(vec![("encoding_format", text("int8"))]),
        Object::new(),
    );
    assert_eq!(int8.get("embeddingTypes"), Some(&list(vec![text("float")])));
    let null = v2::map_openai_params(
        &object(vec![("encoding_format", Json::Null)]),
        Object::new(),
    );
    assert_eq!(null.get("embeddingTypes"), Some(&list(vec![text("float")])));
    let combined = v2::map_openai_params(
        &object(vec![
            ("dimensions", int(256)),
            ("encoding_format", text("base64")),
            ("other", text("ignored")),
        ]),
        Object::new(),
    );
    assert_eq!(combined.get("dimensions"), Some(&int(256)));
    assert_eq!(
        combined.get("embeddingTypes"),
        Some(&list(vec![text("binary")]))
    );
    assert!(!combined.contains_key("other"));
    // map_openai_params mutates the caller's record and hands it back
    let optional = object(vec![("normalize", Json::Bool(true))]);
    let mutated = v2::map_openai_params(&object(vec![("dimensions", int(256))]), optional);
    assert_eq!(mutated.get("normalize"), Some(&Json::Bool(true)));
    assert_eq!(mutated.get("dimensions"), Some(&int(256)));
}

fn assert_v2_request() {
    let inference = object(vec![
        ("dimensions", int(512)),
        ("normalize", Json::Bool(true)),
        ("embeddingTypes", list(vec![text("binary")])),
        ("custom", text("kept")),
    ]);
    let first = v2::transform_request("hello", &inference).unwrap();
    let second = v2::transform_request("", &Object::new()).unwrap();
    assert_eq!(
        first,
        object(vec![
            ("inputText", text("hello")),
            ("dimensions", int(512)),
            ("normalize", Json::Bool(true)),
            ("embeddingTypes", list(vec![text("binary")])),
            ("custom", text("kept")),
        ])
    );
    assert_eq!(second, object(vec![("inputText", text(""))]));
    let collision =
        v2::transform_request("hello", &object(vec![("inputText", text("second"))])).unwrap_err();
    assert!(matches!(
        collision,
        TitanEmbeddingsError::KeywordCollision("inputText")
    ));
    assert_eq!(
        collision.to_string(),
        "got multiple values for keyword argument 'inputText'"
    );
}

fn titan_v2_binary_embedding() {
    assert_v2_binary_preference();
    assert_v2_fallbacks();
    assert_v2_missing_data();
    assert_v2_malformed_by_type();
    assert_v2_embedding_values();
}

fn assert_v2_binary_preference() {
    // binary wins by key presence alone, even next to a float vector
    let mut types = json_object(vec![
        ("binary", list(vec![int(1), int(0), int(7)])),
        ("float", list(vec![float(0.5), float(0.25)])),
    ]);
    let with_binary = object(vec![
        ("inputTextTokenCount", int(3)),
        ("embeddingsByType", Json::Object(types.clone())),
    ]);
    assert_eq!(
        v2::by_type_embedding(&with_binary).unwrap(),
        numbers(&[1, 0, 7])
    );
    // an empty binary vector still counts as present and is not replaced by the float vector
    types.insert(String::from("binary"), list(Vec::new()));
    let empty_binary = object(vec![
        ("inputTextTokenCount", int(1)),
        ("embeddingsByType", Json::Object(types)),
    ]);
    assert_eq!(v2::by_type_embedding(&empty_binary).unwrap(), numbers(&[]));
}

fn assert_v2_fallbacks() {
    // float alone answers, then the legacy embedding column when no usable type key is present
    let cases = [
        (
            object(vec![
                ("inputTextTokenCount", int(3)),
                (
                    "embeddingsByType",
                    json_object(vec![("float", list(vec![float(0.5), float(0.25)]))]),
                ),
            ]),
            floats(&[0.5, 0.25]),
        ),
        (
            object(vec![
                ("inputTextTokenCount", int(3)),
                ("embedding", list(vec![float(0.125), float(0.5)])),
                (
                    "embeddingsByType",
                    json_object(vec![("int8", list(vec![int(4)]))]),
                ),
            ]),
            floats(&[0.125, 0.5]),
        ),
        (
            object(vec![
                ("inputTextTokenCount", int(3)),
                ("embedding", list(vec![float(0.125)])),
                ("embeddingsByType", list(vec![int(4), int(5)])),
            ]),
            floats(&[0.125]),
        ),
        (
            object(vec![("embedding", text("encoded-vector"))]),
            EmbeddingVector::Base64(String::from("encoded-vector")),
        ),
    ];
    for (reply, wanted) in cases {
        assert_eq!(v2::by_type_embedding(&reply).unwrap(), wanted);
    }
}

fn assert_v2_missing_data() {
    let missing = object(vec![("inputTextTokenCount", int(3))]);
    let error = v2::by_type_embedding(&missing).unwrap_err();
    assert!(matches!(error, TitanEmbeddingsError::NoEmbeddingData(_)));
    assert!(
        error
            .to_string()
            .starts_with("No embedding data found in response:")
    );

    let empty_types = object(vec![("embeddingsByType", json_object(Vec::new()))]);
    assert!(matches!(
        v2::by_type_embedding(&empty_types).unwrap_err(),
        TitanEmbeddingsError::NoEmbeddingData(_)
    ));
}

fn assert_v2_malformed_by_type() {
    // "binary" in value runs on None or a number, and both raise before any subscript
    let null_types = object(vec![("embeddingsByType", Json::Null)]);
    assert!(matches!(
        v2::by_type_embedding(&null_types).unwrap_err(),
        TitanEmbeddingsError::NotIterable
    ));
    let number_types = object(vec![("embeddingsByType", int(7))]);
    assert!(matches!(
        v2::by_type_embedding(&number_types).unwrap_err(),
        TitanEmbeddingsError::NotIterable
    ));
    let bool_types = object(vec![("embeddingsByType", Json::Bool(true))]);
    assert!(matches!(
        v2::by_type_embedding(&bool_types).unwrap_err(),
        TitanEmbeddingsError::NotIterable
    ));

    // a string or a list that mentions a type passes the membership test, then fails the name subscript
    let text_types = object(vec![("embeddingsByType", text("floaty"))]);
    assert!(matches!(
        v2::by_type_embedding(&text_types).unwrap_err(),
        TitanEmbeddingsError::NotSubscriptable
    ));
    let harmless_text = object(vec![
        ("embedding", list(vec![float(0.5)])),
        ("embeddingsByType", text("named-by-type")),
    ]);
    assert_eq!(
        v2::by_type_embedding(&harmless_text).unwrap(),
        floats(&[0.5])
    );
    let list_types = object(vec![("embeddingsByType", list(vec![text("binary")]))]);
    assert!(matches!(
        v2::by_type_embedding(&list_types).unwrap_err(),
        TitanEmbeddingsError::NotSubscriptable
    ));
}

fn assert_v2_embedding_values() {
    // a chosen key whose value is null or not a vector fails the Embedding construction
    let null_binary = object(vec![(
        "embeddingsByType",
        json_object(vec![("binary", Json::Null)]),
    )]);
    assert!(matches!(
        v2::by_type_embedding(&null_binary).unwrap_err(),
        TitanEmbeddingsError::MalformedEmbedding("embeddingsByType")
    ));
    let mixed_float = object(vec![(
        "embeddingsByType",
        json_object(vec![("float", list(vec![float(0.5), text("x")]))]),
    )]);
    assert!(matches!(
        v2::by_type_embedding(&mixed_float).unwrap_err(),
        TitanEmbeddingsError::MalformedEmbedding("embeddingsByType")
    ));
    let object_embedding = object(vec![("embedding", json_object(vec![("a", int(1))]))]);
    assert!(matches!(
        v2::by_type_embedding(&object_embedding).unwrap_err(),
        TitanEmbeddingsError::MalformedEmbedding("embedding")
    ));
}

fn titan_multimodal_image() {
    assert_multimodal_mapping();
    assert_multimodal_text_request();
    assert_multimodal_image_request();
    assert_multimodal_overrides();
}

fn assert_multimodal_mapping() {
    assert_eq!(multimodal::SUPPORTED_OPENAI_PARAMS, &["dimensions"]);
    assert_eq!(
        multimodal::map_openai_params(&object(vec![("dimensions", int(1024))]), Object::new()),
        object(vec![(
            "embeddingConfig",
            json_object(vec![("outputEmbeddingLength", int(1024))])
        )])
    );
    assert_eq!(
        multimodal::map_openai_params(&object(vec![("other", int(1))]), Object::new()),
        Object::new()
    );
}

fn assert_multimodal_text_request() {
    assert_eq!(
        multimodal::transform_request("a plain description", &Object::new()),
        object(vec![("inputText", text("a plain description"))])
    );
    // a data URL B183 refuses falls back to text, and no URL is fetched either way
    let malformed = "data:image/png;base64,not base64!";
    assert_eq!(
        multimodal::transform_request(malformed, &Object::new()),
        object(vec![("inputText", text(malformed))])
    );
    let https = "https://example.com/image.png";
    assert_eq!(
        multimodal::transform_request(https, &Object::new()),
        object(vec![("inputText", text(https))])
    );
    let inference = object(vec![(
        "embeddingConfig",
        json_object(vec![("outputEmbeddingLength", int(256))]),
    )]);
    assert_eq!(
        multimodal::transform_request("a plain description", &inference),
        object(vec![
            ("inputText", text("a plain description")),
            (
                "embeddingConfig",
                json_object(vec![("outputEmbeddingLength", int(256))])
            ),
        ])
    );
}

fn assert_multimodal_image_request() {
    let data_url = "data:image/png;base64,aGVsbG8=";
    assert_eq!(
        multimodal::transform_request(data_url, &Object::new()),
        object(vec![("inputImage", text("aGVsbG8="))])
    );
}

fn assert_multimodal_overrides() {
    // inference keys are copied after the choice, so a caller can replace either key or add the other one
    let overrides = object(vec![
        ("inputText", text("replacement text")),
        ("inputImage", text("replacement-image")),
        ("custom", text("kept")),
    ]);
    let request = multimodal::transform_request("a plain description", &overrides);
    let names: Vec<&str> = request.keys().map(String::as_str).collect();
    assert_eq!(names, ["inputText", "inputImage", "custom"]);
    assert_eq!(request["inputText"], text("replacement text"));
    assert_eq!(request["inputImage"], text("replacement-image"));
    assert_eq!(request["custom"], text("kept"));
}

fn cohere_v3_input_type() {
    assert_cohere_mapping();
    assert_cohere_requests();
}

fn assert_cohere_mapping() {
    assert_eq!(
        cohere::SUPPORTED_OPENAI_PARAMS,
        &["encoding_format", "dimensions"]
    );
    let cases = [
        (
            object(vec![("encoding_format", text("base64"))]),
            "embedding_types",
            list(vec![text("float")]),
        ),
        // a list maps elementwise, and only base64 turns into float
        (
            object(vec![(
                "encoding_format",
                list(vec![text("base64"), text("int8"), text("uint8")]),
            )]),
            "embedding_types",
            list(vec![text("float"), text("int8"), text("uint8")]),
        ),
        (
            object(vec![("encoding_format", list(Vec::new()))]),
            "embedding_types",
            list(Vec::new()),
        ),
        // any other value is a singleton, and an explicit null survives as one
        (
            object(vec![("encoding_format", Json::Null)]),
            "embedding_types",
            list(vec![Json::Null]),
        ),
        (
            object(vec![("dimensions", int(256))]),
            "output_dimension",
            int(256),
        ),
    ];
    for (non_default, name, expected) in cases {
        let mapped = cohere::map_openai_params(&non_default, Object::new());
        assert_eq!(mapped.get(name), Some(&expected));
    }
    let combined = cohere::map_openai_params(
        &object(vec![
            ("encoding_format", text("base64")),
            ("dimensions", int(256)),
            ("other", text("ignored")),
        ]),
        Object::new(),
    );
    assert_eq!(
        combined.get("embedding_types"),
        Some(&list(vec![text("float")]))
    );
    assert_eq!(combined.get("output_dimension"), Some(&int(256)));
    assert!(!combined.contains_key("other"));
}

fn assert_cohere_requests() {
    let cases = [
        (
            "embed-english-v3.0",
            vec!["hello", "world"],
            Object::new(),
            object(vec![
                ("input_type", text("search_document")),
                ("texts", list(vec![text("hello"), text("world")])),
            ]),
        ),
        // the delegated parent keeps only the fields CohereEmbeddingRequest declares
        (
            "embed-english-v3.0",
            vec!["hello"],
            object(vec![
                ("truncate", text("END")),
                ("embedding_types", list(vec![text("float")])),
                ("output_dimension", int(256)),
                ("custom", text("dropped")),
                ("model", text("dropped-too")),
            ]),
            object(vec![
                ("input_type", text("search_document")),
                ("texts", list(vec![text("hello")])),
                ("truncate", text("END")),
                ("embedding_types", list(vec![text("float")])),
                ("output_dimension", int(256)),
            ]),
        ),
        (
            "embed-english-v3.0",
            vec!["data:image/png;base64,aGVsbG8="],
            Object::new(),
            object(vec![
                ("input_type", text("image")),
                ("images", list(vec![text("data:image/png;base64,aGVsbG8=")])),
            ]),
        ),
        // the last input decides for the whole batch
        (
            "embed-english-v3.0",
            vec!["hello", "data:image/png;base64,aGVsbG8="],
            Object::new(),
            object(vec![
                ("input_type", text("image")),
                (
                    "images",
                    list(vec![text("hello"), text("data:image/png;base64,aGVsbG8=")]),
                ),
            ]),
        ),
        // inference input_type replaces the delegated default, as it is copied after the classification
        (
            "embed-english-v3.0",
            vec!["hello"],
            object(vec![("input_type", text("search_query"))]),
            object(vec![
                ("input_type", text("search_query")),
                ("texts", list(vec![text("hello")])),
            ]),
        ),
        (
            "embed-english-v3.0",
            Vec::new(),
            Object::new(),
            object(vec![
                ("input_type", text("search_document")),
                ("texts", list(Vec::new())),
            ]),
        ),
    ];
    for (model, texts, inference, expected) in cases {
        let inputs: Vec<String> = texts.into_iter().map(String::from).collect();
        assert_eq!(
            cohere::transform_request(model, &inputs, &inference),
            expected
        );
    }
}

fn token_count_response() {
    assert_g1_response();
    assert_v2_response();
    assert_multimodal_response();
    assert_response_failures();
}

fn assert_g1_response() {
    let replies = [
        object(vec![
            ("embedding", list(vec![float(0.1), float(0.2)])),
            ("inputTextTokenCount", int(5)),
            ("extra", text("ignored")),
        ]),
        object(vec![
            ("embedding", list(vec![int(1), int(0)])),
            ("inputTextTokenCount", int(7)),
        ]),
    ];
    let result =
        response::transform_response(&replies, "titan-embed-text-v1", response::plain_embedding)
            .unwrap();
    assert_eq!(result.object, "list");
    assert_eq!(result.model, "titan-embed-text-v1");
    assert_eq!(result.data.len(), 2);
    assert_eq!(result.data[0].object, "embedding");
    assert_eq!(result.data[0].index, 0);
    assert_eq!(result.data[0].embedding, floats(&[0.1, 0.2]));
    assert_eq!(result.data[1].index, 1);
    // an integer vector stays integer, not a base64 string
    assert_eq!(result.data[1].embedding, numbers(&[1, 0]));
    assert_eq!(result.usage.prompt_tokens, 12);
    assert_eq!(result.usage.completion_tokens, 0);
    assert_eq!(result.usage.total_tokens, 12);
    assert_eq!(result.usage.prompt_tokens_details, None);
}

fn assert_v2_response() {
    let reply = object(vec![
        ("inputTextTokenCount", int(4)),
        (
            "embeddingsByType",
            json_object(vec![("binary", list(vec![int(255), int(0)]))]),
        ),
    ]);
    let result =
        response::transform_response(&[reply], "titan-embed-text-v2", v2::by_type_embedding)
            .unwrap();
    assert_eq!(result.object, "list");
    assert_eq!(result.model, "titan-embed-text-v2");
    assert_eq!(result.data.len(), 1);
    let record = &result.data[0];
    assert_eq!((record.index, &record.embedding), (0, &numbers(&[255, 0])));
    assert_eq!(
        (
            result.usage.prompt_tokens,
            result.usage.completion_tokens,
            result.usage.total_tokens,
        ),
        (4, 0, 4)
    );
    assert_eq!(result.usage.prompt_tokens_details, None);
}

fn multimodal_replies() -> Vec<Object> {
    vec![object(vec![
        ("embedding", list(vec![float(0.5), float(0.25)])),
        ("inputTextTokenCount", int(6)),
    ])]
}

fn assert_multimodal_response() {
    let batch = [
        object(vec![("inputImage", text("aGVsbG8="))]),
        object(vec![("inputText", text("hello"))]),
        object(vec![("inputImage", Json::Null)]),
    ];
    let result =
        multimodal::transform_response(&multimodal_replies(), "titan-embed-image", Some(&batch))
            .unwrap();
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.usage.prompt_tokens, 6);
    assert_eq!(
        result.usage.prompt_tokens_details,
        Some(TitanPromptTokensDetails { image_count: 2 })
    );

    // the count is independent of the reply count and stops at zero images
    let no_images = multimodal::transform_response(
        &[],
        "titan-embed-image",
        Some(&[object(vec![("inputText", text("hello"))])]),
    )
    .unwrap();
    assert_eq!(no_images.data, Vec::new());
    assert_eq!(no_images.usage.prompt_tokens, 0);
    assert_eq!(no_images.usage.prompt_tokens_details, None);

    let images_without_replies = multimodal::transform_response(
        &[],
        "titan-embed-image",
        Some(&[object(vec![("inputImage", text("aGVsbG8="))])]),
    )
    .unwrap();
    assert_eq!(
        images_without_replies.usage.prompt_tokens_details,
        Some(TitanPromptTokensDetails { image_count: 1 })
    );

    let no_batch =
        multimodal::transform_response(&multimodal_replies(), "titan-embed-image", None).unwrap();
    assert_eq!(no_batch.usage.prompt_tokens_details, None);
}

fn assert_response_failures() {
    let empty: TitanEmbeddingResponse =
        response::transform_response(&[], "titan-embed-text-v1", response::plain_embedding)
            .unwrap();
    assert_eq!(empty.data, Vec::new());
    assert_eq!(empty.usage.prompt_tokens, 0);
    assert_eq!(empty.usage.total_tokens, 0);

    let missing_token = object(vec![("embedding", list(vec![float(0.5)]))]);
    assert!(matches!(
        response::transform_response(&[missing_token], "m", response::plain_embedding).unwrap_err(),
        TitanEmbeddingsError::MissingField("inputTextTokenCount")
    ));

    let malformed_token = object(vec![
        ("embedding", list(vec![float(0.5)])),
        ("inputTextTokenCount", text("many")),
    ]);
    assert!(matches!(
        response::transform_response(&[malformed_token], "m", response::plain_embedding)
            .unwrap_err(),
        TitanEmbeddingsError::MalformedTokenCount
    ));

    let negative_token = object(vec![
        ("embedding", list(vec![float(0.5)])),
        ("inputTextTokenCount", int(-1)),
    ]);
    assert!(matches!(
        response::transform_response(&[negative_token], "m", response::plain_embedding)
            .unwrap_err(),
        TitanEmbeddingsError::MalformedTokenCount
    ));

    let missing_embedding = object(vec![("inputTextTokenCount", int(1))]);
    assert!(matches!(
        response::transform_response(&[missing_embedding], "m", response::plain_embedding)
            .unwrap_err(),
        TitanEmbeddingsError::MissingField("embedding")
    ));
}
