#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use rstest::rstest;
use serde_json::Number;
use tokio::runtime::{Builder, Runtime};

use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::batch_embeddings::classify::{BatchRecordKind, classify_batch_record};
use crate::batch_embeddings::error::BatchEmbeddingError;
use crate::batch_embeddings::input::coerce_input_to_string;
use crate::batch_embeddings::mapping::map_embedding_to_bedrock;
use crate::batch_embeddings::titan::{is_titan_v2_embed_model, lookup_provider_specific_field};
use crate::bedrock_model_capabilities::ModelInfoLookup;

const TITAN_V2_ENTRY: &str = r#"{"litellm_provider":"bedrock","mode":"embedding","provider_specific_entry":{"bedrock_invocation_schema":"titan_v2"}}"#;
const COHERE_ENTRY: &str = r#"{"litellm_provider":"bedrock","mode":"embedding","provider_specific_entry":{"bedrock_invocation_schema":"cohere_v3"}}"#;
const EMPTY_SCHEMA_ENTRY: &str = r#"{"litellm_provider":"bedrock","mode":"embedding","provider_specific_entry":{"bedrock_invocation_schema":""}}"#;
const NUMBER_SCHEMA_ENTRY: &str = r#"{"litellm_provider":"bedrock","mode":"embedding","provider_specific_entry":{"bedrock_invocation_schema":3}}"#;
const NO_SCHEMA_ENTRY: &str = r#"{"litellm_provider":"bedrock","mode":"embedding"}"#;
const TEXT_SCHEMA_ENTRY: &str =
    r#"{"litellm_provider":"bedrock","mode":"embedding","provider_specific_entry":"flat"}"#;
const NULL_SCHEMA_ENTRY: &str =
    r#"{"litellm_provider":"bedrock","mode":"embedding","provider_specific_entry":null}"#;

const TITAN_MODEL: &str = "amazon.titan-embed-text-v2:0";

const MULTIPLE_INPUTS: &str = "Bedrock batch embedding requires one input per JSONL record.";
const TOKENIZED_INPUT: &str =
    "Bedrock Titan v2 batch embedding does not support pre-tokenized integer inputs.";
const INPUT_NOT_STRING: &str =
    "Bedrock batch embedding `input` must be a string (or a single-element list of strings).";

fn parsed(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture is a JSON object")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn int(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite float"))
}

fn list(values: Vec<Json>) -> Json {
    Json::List(values)
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the fixture runtime builds")
}

#[derive(Debug, thiserror::Error)]
#[error("the fixture registry holds no entry for this model")]
struct UnmappedModel;

struct Registry {
    entries: BTreeMap<String, ModelInfo>,
}

impl ModelInfoLookup for Registry {
    type Error = UnmappedModel;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, UnmappedModel>> + Send {
        ready(self.entries.get(model).cloned().ok_or(UnmappedModel))
    }
}

fn registry(entries: &[(&str, &str)]) -> Registry {
    Registry {
        entries: entries
            .iter()
            .map(|(model, source)| ((*model).to_owned(), info(source)))
            .collect(),
    }
}

fn info(source: &str) -> ModelInfo {
    ModelInfo {
        fields: serde_json::from_str(source).expect("the fixture entry is a JSON object"),
    }
}

fn titan(runtime: &Runtime, lookup: &Registry, model: &str) -> bool {
    runtime.block_on(is_titan_v2_embed_model(model, lookup))
}

fn schema(runtime: &Runtime, lookup: &Registry, model: &str) -> Option<String> {
    runtime.block_on(lookup_provider_specific_field(
        model,
        "bedrock_invocation_schema",
        lookup,
    ))
}

fn mapped(runtime: &Runtime, lookup: &Registry, source: &str) -> Object {
    runtime
        .block_on(map_embedding_to_bedrock(
            &parsed(source),
            TITAN_MODEL,
            lookup,
        ))
        .expect("the fixture body maps")
}

fn refuse(runtime: &Runtime, lookup: &Registry, model: &str, source: &str) -> BatchEmbeddingError {
    runtime
        .block_on(map_embedding_to_bedrock(&parsed(source), model, lookup))
        .expect_err("the fixture body is refused")
}

fn message_of(result: Result<String, BatchEmbeddingError>) -> String {
    result
        .expect_err("the fixture input is refused")
        .to_string()
}

#[rstest]
#[case::url_before_body_classification(url_before_body_classification)]
#[case::no_url_body_classification(no_url_body_classification)]
#[case::titan_v2_registry_schema(titan_v2_registry_schema)]
#[case::titan_v2_name_boundary(titan_v2_name_boundary)]
#[case::single_input_coercion(single_input_coercion)]
#[case::invalid_multi_input(invalid_multi_input)]
#[case::dimensions_normalize_fields(dimensions_normalize_fields)]
#[case::unsupported_embedding_model(unsupported_embedding_model)]
fn batch_embeddings_cases(#[case] check: fn()) {
    check();
}

fn url_before_body_classification() {
    assert_supported_urls();
    assert_other_urls();
}

fn assert_supported_urls() {
    let cases = [
        (
            r#"{"url":"/v1/embeddings","body":{"messages":[]}}"#,
            BatchRecordKind::Embedding,
        ),
        (
            r#"{"url":"/v1/completions","body":{"messages":[]}}"#,
            BatchRecordKind::TextCompletion,
        ),
        (
            r#"{"url":"/v1/responses","body":{"messages":[]}}"#,
            BatchRecordKind::Responses,
        ),
        (r#"{"url":"/v1/embeddings"}"#, BatchRecordKind::Embedding),
        (
            r#"{"url":"/v1/embeddings","body":"x"}"#,
            BatchRecordKind::Embedding,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(classify_batch_record(&parsed(source)), expected, "{source}");
    }
}

fn assert_other_urls() {
    let cases = [
        (
            r#"{"url":"/embeddings","body":{"input":"x"}}"#,
            BatchRecordKind::Chat,
        ),
        (
            r#"{"url":"/v1/embeddings?beta=1","body":{"input":"x"}}"#,
            BatchRecordKind::Chat,
        ),
        (
            r#"{"url":" /v1/embeddings","body":{"input":"x"}}"#,
            BatchRecordKind::Chat,
        ),
        (
            r#"{"url":"/v1/Embeddings","body":{"input":"x"}}"#,
            BatchRecordKind::Chat,
        ),
        (r#"{"url":123,"body":{"input":"x"}}"#, BatchRecordKind::Chat),
        (
            r#"{"url":["/v1/embeddings"],"body":{"input":"x"}}"#,
            BatchRecordKind::Chat,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(classify_batch_record(&parsed(source)), expected, "{source}");
    }
}

fn no_url_body_classification() {
    assert_body_shapes();
    assert_body_keys();
}

fn assert_body_shapes() {
    let cases = [
        (
            r#"{"body":{"messages":[{"role":"user","content":"hi"}]}}"#,
            BatchRecordKind::Chat,
        ),
        (
            r#"{"url":null,"body":"not-a-mapping"}"#,
            BatchRecordKind::Chat,
        ),
        (r#"{"url":null}"#, BatchRecordKind::Chat),
        (r#"{"url":null,"body":[1,2]}"#, BatchRecordKind::Chat),
        (r#"{"url":null,"body":null}"#, BatchRecordKind::Chat),
        (r#"{"url":null,"body":{}}"#, BatchRecordKind::Chat),
    ];
    for (source, expected) in cases {
        assert_eq!(classify_batch_record(&parsed(source)), expected, "{source}");
    }
}

fn assert_body_keys() {
    let cases = [
        (
            r#"{"url":null,"body":{"prompt":"p"}}"#,
            BatchRecordKind::TextCompletion,
        ),
        (
            r#"{"url":"","body":{"input":"x"}}"#,
            BatchRecordKind::Embedding,
        ),
        (
            r#"{"url":null,"body":{"prompt":null,"input":null}}"#,
            BatchRecordKind::TextCompletion,
        ),
        (
            r#"{"url":null,"body":{"messages":null,"prompt":"p"}}"#,
            BatchRecordKind::Chat,
        ),
        (
            r#"{"url":null,"body":{"input":null}}"#,
            BatchRecordKind::Embedding,
        ),
        (r#"{"url":null,"body":{"other":1}}"#, BatchRecordKind::Chat),
    ];
    for (source, expected) in cases {
        assert_eq!(classify_batch_record(&parsed(source)), expected, "{source}");
    }
}

fn titan_v2_registry_schema() {
    let lookup = registry(&[
        ("amazon.titan-embed-text-v2:0", TITAN_V2_ENTRY),
        ("amazon.titan-embed-text-v2:9", COHERE_ENTRY),
        ("amazon.titan-embed-text-v2:1", EMPTY_SCHEMA_ENTRY),
        ("amazon.titan-embed-text-v2:2", NUMBER_SCHEMA_ENTRY),
        ("amazon.titan-embed-text-v2:3", NO_SCHEMA_ENTRY),
        ("amazon.titan-embed-text-v2:5", TEXT_SCHEMA_ENTRY),
        ("cohere.embed-multilingual-v3", NULL_SCHEMA_ENTRY),
    ]);
    assert_registry_schemas(&lookup);
    assert_registry_titan(&lookup);
}

fn assert_registry_schemas(lookup: &Registry) {
    let runtime = runtime();
    let cases = [
        ("amazon.titan-embed-text-v2:0", Some("titan_v2")),
        ("amazon.titan-embed-text-v2:9", Some("cohere_v3")),
        ("amazon.titan-embed-text-v2:1", None),
        ("amazon.titan-embed-text-v2:2", None),
        ("amazon.titan-embed-text-v2:3", None),
        ("amazon.titan-embed-text-v2:5", None),
        ("cohere.embed-multilingual-v3", None),
        ("amazon.titan-embed-text-v4:0", None),
    ];
    for (model, expected) in cases {
        let found = schema(&runtime, lookup, model);
        assert_eq!(found.as_deref(), expected, "{model}");
    }
}

fn assert_registry_titan(lookup: &Registry) {
    let runtime = runtime();
    let cases = [
        // a resolved id trusts its own schema, so another schema never falls back to the name
        ("amazon.titan-embed-text-v2:0", true),
        ("amazon.titan-embed-text-v2:9", false),
        // an empty, nonstring or absent schema and an absent entry all permit the name fallback
        ("amazon.titan-embed-text-v2:1", true),
        ("amazon.titan-embed-text-v2:2", true),
        ("amazon.titan-embed-text-v2:3", true),
        ("amazon.titan-embed-text-v2:5", true),
        ("cohere.embed-multilingual-v3", false),
        ("amazon.titan-embed-text-v4:0", false),
        ("amazon.titan-embed-text-v1", false),
    ];
    for (model, expected) in cases {
        assert_eq!(titan(&runtime, lookup, model), expected, "{model}");
    }
}

fn titan_v2_name_boundary() {
    // every lookup fails here, so each answer comes from the id alone
    let lookup = registry(&[]);
    let runtime = runtime();
    let cases = [
        ("amazon.titan-embed-text-v2:0", true),
        ("us.amazon.titan-embed-text-v2:0", true),
        (
            "arn:aws:bedrock:us-east-1:123:inference-profile/us.amazon.titan-embed-text-v2:0",
            true,
        ),
        ("bedrock/amazon.titan-embed-text-v2:0", true),
        ("bedrock/bedrock/amazon.titan-embed-text-v2:0", true),
        ("amazon.titan-embed-text-v20", false),
        ("amazon.titan-embed-text-v2-experimental", false),
        ("amazon.titan-embed-text-v2", true),
        ("AMAZON.TITAN-EMBED-TEXT-V2:0", true),
        (
            "amazon.titan-embed-text-v2-experimental/amazon.titan-embed-text-v2:0",
            false,
        ),
        ("amazon.titan-embed-text-v2/foo", true),
        ("titan-embed-text-v2", true),
        ("notitan-embed-text-v2:0", true),
        ("amazon.titan-embed-text-v1", false),
    ];
    for (model, expected) in cases {
        assert_eq!(titan(&runtime, &lookup, model), expected, "{model}");
    }
}

fn single_input_coercion() {
    let accepted = [
        (text("hello"), "hello"),
        (text(""), ""),
        (list(vec![text("hello")]), "hello"),
        (list(vec![text("")]), ""),
    ];
    for (raw, expected) in accepted {
        let coerced = coerce_input_to_string(Some(&raw), "m");
        assert_eq!(coerced.expect("the input is one string"), expected);
    }
    let missing = "Embedding batch record is missing required `input` field: model=m";
    assert_eq!(message_of(coerce_input_to_string(None, "m")), missing);
    assert_eq!(
        message_of(coerce_input_to_string(Some(&Json::Null), "m")),
        missing
    );
}

fn invalid_multi_input() {
    assert_multiple_inputs();
    assert_tokenized_and_typed_inputs();
}

fn assert_multiple_inputs() {
    let cases = [
        (
            list(vec![text("a"), text("b")]),
            format!(
                "{MULTIPLE_INPUTS} Got a list with 2 items for model=m; emit one JSONL line per input string instead."
            ),
        ),
        (
            list(Vec::new()),
            format!(
                "{MULTIPLE_INPUTS} Got a list with 0 items for model=m; emit one JSONL line per input string instead."
            ),
        ),
        (
            list(vec![int(1), int(2)]),
            format!(
                "{MULTIPLE_INPUTS} Got a list with 2 items for model=m; emit one JSONL line per input string instead."
            ),
        ),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            message_of(coerce_input_to_string(Some(&raw), "m")),
            expected
        );
    }
}

fn assert_tokenized_and_typed_inputs() {
    let cases = [
        (
            list(vec![list(vec![int(1)])]),
            format!("{TOKENIZED_INPUT} Pass `input` as a string (model=m)."),
        ),
        (
            Json::Bool(true),
            format!("{TOKENIZED_INPUT} Pass `input` as a string (model=m)."),
        ),
        (
            int(5),
            format!("{TOKENIZED_INPUT} Pass `input` as a string (model=m)."),
        ),
        (
            float(1.5),
            format!("{INPUT_NOT_STRING} Got type float for model=m."),
        ),
        (
            Json::Object(parsed(r#"{"a":1}"#)),
            format!("{INPUT_NOT_STRING} Got type dict for model=m."),
        ),
        (
            list(vec![Json::Null]),
            format!("{INPUT_NOT_STRING} Got type NoneType for model=m."),
        ),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            message_of(coerce_input_to_string(Some(&raw), "m")),
            expected
        );
    }
}

fn dimensions_normalize_fields() {
    let lookup = registry(&[(TITAN_MODEL, TITAN_V2_ENTRY)]);
    let runtime = runtime();
    assert_dimension_mapping(&runtime, &lookup);
    assert_encoding_mapping(&runtime, &lookup);
}

fn assert_dimension_mapping(runtime: &Runtime, lookup: &Registry) {
    let cases = [
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello"}"#,
            r#"{"inputText":"hello"}"#,
        ),
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","dimensions":512}"#,
            r#"{"inputText":"hello","dimensions":512}"#,
        ),
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","dimensions":null}"#,
            r#"{"inputText":"hello","dimensions":null}"#,
        ),
        (
            r#"{"model":"ignored-by-the-mapping","input":["hello"],"dimensions":1024,"encoding_format":"base64","normalize":true,"user":"someone","metadata":{"k":"v"}}"#,
            r#"{"inputText":"hello","dimensions":1024,"embeddingTypes":["binary"]}"#,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            mapped(runtime, lookup, source),
            parsed(expected),
            "{source}"
        );
    }
}

fn assert_encoding_mapping(runtime: &Runtime, lookup: &Registry) {
    let cases = [
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","encoding_format":"base64"}"#,
            r#"{"inputText":"hello","embeddingTypes":["binary"]}"#,
        ),
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","encoding_format":"float"}"#,
            r#"{"inputText":"hello","embeddingTypes":["float"]}"#,
        ),
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","encoding_format":"hex"}"#,
            r#"{"inputText":"hello","embeddingTypes":["float"]}"#,
        ),
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":""}"#,
            r#"{"inputText":""}"#,
        ),
        (
            r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","metadata":null}"#,
            r#"{"inputText":"hello"}"#,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            mapped(runtime, lookup, source),
            parsed(expected),
            "{source}"
        );
    }
}

fn unsupported_embedding_model() {
    let lookup = registry(&[
        ("cohere.embed-english-v3", TEXT_SCHEMA_ENTRY),
        ("amazon.titan-embed-text-v1", NO_SCHEMA_ENTRY),
        ("amazon.titan-embed-text-v2:9", COHERE_ENTRY),
    ]);
    let runtime = runtime();
    // the first record has no input at all, so the model is refused before the body is inspected
    let cases = [
        (
            "amazon.titan-embed-text-v1",
            r#"{"model":"amazon.titan-embed-text-v1"}"#,
        ),
        (
            "amazon.titan-embed-text-v2:9",
            r#"{"model":"amazon.titan-embed-text-v2:9","input":"hello"}"#,
        ),
        (
            "cohere.embed-english-v3",
            r#"{"model":"cohere.embed-english-v3","input":"hello"}"#,
        ),
    ];
    for (model, source) in cases {
        let unknown = refuse(&runtime, &lookup, model, source);
        let expected = format!(
            "Bedrock batch embedding currently supports only Amazon Titan Text Embeddings V2 (model id contains 'titan-embed-text-v2'). Got model=\"{model}\". Track other embedding models in https://github.com/BerriAI/litellm/issues."
        );
        assert_eq!(unknown.to_string(), expected, "{model}");
    }
}
