#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_prompts::arguments::json::Json;

use crate::batch_embeddings::classify::{BatchRecord, BatchRecordKind, classify_batch_record};
use crate::batch_embeddings::error::BatchEmbeddingError;
use crate::batch_embeddings::input::{EmbeddingInput, coerce_input_to_string};
use crate::batch_embeddings::mapping::{
    EmbeddingRequest, InvokeModelBody, map_embedding_to_bedrock,
};
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
// a registered id that carries the Titan v2 schema without the name marker
const TITAN_ALIAS: &str = "custom-embedding";

const MULTIPLE_INPUTS: &str = "Bedrock batch embedding requires one input per JSONL record.";
const TOKENIZED_INPUT: &str =
    "Bedrock Titan v2 batch embedding does not support pre-tokenized integer inputs.";
const INPUT_NOT_STRING: &str =
    "Bedrock batch embedding `input` must be a string (or a single-element list of strings).";

// a url naming an endpoint wins over the body; only an absent, null or empty one falls through
const URL_CASES: &[(&str, BatchRecordKind)] = &[
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

// without a usable url a non-object body is chat, then key presence decides, null included
const BODY_CASES: &[(&str, BatchRecordKind)] = &[
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

const REGISTRY_ENTRIES: &[(&str, &str)] = &[
    ("amazon.titan-embed-text-v2:0", TITAN_V2_ENTRY),
    ("amazon.titan-embed-text-v2:9", COHERE_ENTRY),
    ("amazon.titan-embed-text-v2:1", EMPTY_SCHEMA_ENTRY),
    ("amazon.titan-embed-text-v2:2", NUMBER_SCHEMA_ENTRY),
    ("amazon.titan-embed-text-v2:3", NO_SCHEMA_ENTRY),
    ("amazon.titan-embed-text-v2:5", TEXT_SCHEMA_ENTRY),
    ("cohere.embed-multilingual-v3", NULL_SCHEMA_ENTRY),
    (TITAN_ALIAS, TITAN_V2_ENTRY),
];

// one row per id, so the resolved schema and the flag it drives stay side by side
const REGISTRY_CASES: &[(&str, Option<&str>, bool)] = &[
    ("amazon.titan-embed-text-v2:0", Some("titan_v2"), true),
    // a resolved id trusts its own schema, so another schema never falls back to the name
    ("amazon.titan-embed-text-v2:9", Some("cohere_v3"), false),
    // an empty, nonstring or absent schema and an absent entry all permit the name fallback
    ("amazon.titan-embed-text-v2:1", None, true),
    ("amazon.titan-embed-text-v2:2", None, true),
    ("amazon.titan-embed-text-v2:3", None, true),
    ("amazon.titan-embed-text-v2:5", None, true),
    ("cohere.embed-multilingual-v3", None, false),
    ("amazon.titan-embed-text-v4:0", None, false),
    ("amazon.titan-embed-text-v1", None, false),
    // a registered alias is detected through its schema even without the marker in its name
    (TITAN_ALIAS, Some("titan_v2"), true),
];

// every lookup fails for these ids, so each answer comes from the name alone
const NAME_CASES: &[(&str, bool)] = &[
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

const MAPPING_CASES: &[(&str, &str)] = &[
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
        r#"{"model":"amazon.titan-embed-text-v2:0","input":"hello","encoding_format":null}"#,
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

const UNSUPPORTED_ENTRIES: &[(&str, &str)] = &[
    ("cohere.embed-english-v3", TEXT_SCHEMA_ENTRY),
    ("amazon.titan-embed-text-v1", NO_SCHEMA_ENTRY),
    ("amazon.titan-embed-text-v2:9", COHERE_ENTRY),
];

// the first record has no input at all, so the model is refused before the body is inspected
const UNSUPPORTED_CASES: &[(&str, &str)] = &[
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

fn record(source: &str) -> BatchRecord {
    serde_json::from_str(source).expect("the fixture is a batch record")
}

fn embedding_request(source: &str) -> EmbeddingRequest {
    serde_json::from_str(source).expect("the fixture body is an embeddings request")
}

fn input(source: &str) -> EmbeddingInput {
    serde_json::from_str(source).expect("the fixture is one embeddings input")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the fixture runtime builds")
}

fn multiple_inputs(items: usize) -> String {
    format!(
        "{MULTIPLE_INPUTS} Got a list with {items} items for model=m; emit one JSONL line per input string instead."
    )
}

fn tokenized() -> String {
    format!("{TOKENIZED_INPUT} Pass `input` as a string (model=m).")
}

fn not_a_string(found: &str) -> String {
    format!("{INPUT_NOT_STRING} Got type {found} for model=m.")
}

fn unsupported_message(model: &str) -> String {
    format!(
        "Bedrock batch embedding currently supports only Amazon Titan Text Embeddings V2 (model id contains 'titan-embed-text-v2'). Got model='{model}'. Track other embedding models in https://github.com/BerriAI/litellm/issues."
    )
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

fn body_json(body: &InvokeModelBody) -> Json {
    let value = serde_json::to_value(body).expect("the mapped body serializes");
    serde_json::from_value(value).expect("the mapped body is JSON")
}

fn map(
    runtime: &Runtime,
    lookup: &Registry,
    model: &str,
    source: &str,
) -> Result<Json, BatchEmbeddingError> {
    let mapped = runtime.block_on(map_embedding_to_bedrock(
        &embedding_request(source),
        model,
        lookup,
    ));
    mapped.map(|body| body_json(&body))
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
    assert_classified(URL_CASES);
}

fn no_url_body_classification() {
    assert_classified(BODY_CASES);
}

fn assert_classified(cases: &[(&str, BatchRecordKind)]) {
    for (source, expected) in cases {
        assert_eq!(
            classify_batch_record(&record(source)),
            *expected,
            "{source}"
        );
    }
}

fn titan_v2_registry_schema() {
    let lookup = registry(REGISTRY_ENTRIES);
    let runtime = runtime();
    for (model, expected_schema, expected_titan) in REGISTRY_CASES {
        assert_eq!(
            schema(&runtime, &lookup, model).as_deref(),
            *expected_schema,
            "{model}"
        );
        assert_eq!(titan(&runtime, &lookup, model), *expected_titan, "{model}");
    }
    // the alias has no marker in its name, so only the resolved schema can accept it
    assert_eq!(
        map(
            &runtime,
            &lookup,
            TITAN_ALIAS,
            r#"{"model":"custom-embedding","input":"hello"}"#
        )
        .expect("the fixture body maps"),
        json(r#"{"inputText":"hello"}"#),
        "{TITAN_ALIAS}"
    );
}

fn titan_v2_name_boundary() {
    let lookup = registry(&[]);
    let runtime = runtime();
    for (model, expected) in NAME_CASES {
        assert_eq!(titan(&runtime, &lookup, model), *expected, "{model}");
    }
}

fn single_input_coercion() {
    let accepted = [
        (input(r#""hello""#), "hello"),
        (input(r#""""#), ""),
        (input(r#"["hello"]"#), "hello"),
        (input(r#"[""]"#), ""),
    ];
    for (raw, expected) in accepted {
        let coerced = coerce_input_to_string(Some(&raw), "m");
        assert_eq!(coerced.expect("the input is one string"), expected);
    }
    let missing = "Embedding batch record is missing required `input` field: model=m";
    assert_eq!(message_of(coerce_input_to_string(None, "m")), missing);
    // a null input collapses to None at the typed boundary and reads as missing, as `.get` did
    let null_body = embedding_request(r#"{"input":null}"#);
    assert_eq!(
        message_of(coerce_input_to_string(null_body.input.as_ref(), "m")),
        missing
    );
}

fn invalid_multi_input() {
    let cases = [
        (input(r#"["a","b"]"#), multiple_inputs(2)),
        (input("[]"), multiple_inputs(0)),
        (input("[1,2]"), multiple_inputs(2)),
        (input("[[1]]"), tokenized()),
        (input("true"), tokenized()),
        (input("5"), tokenized()),
        // a negative int is still an int even though it is not a u64
        (input("-1"), tokenized()),
        (input("1.5"), not_a_string("float")),
        (input("1e5"), not_a_string("float")),
        (input("1E5"), not_a_string("float")),
        // a whole number past the i64/u64 range is still an int, so it reads as tokens
        (input("123456789012345678901234567890"), tokenized()),
        (input(r#"{"a":1}"#), not_a_string("dict")),
        (input("[null]"), not_a_string("NoneType")),
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
    assert_mapped(&runtime, &lookup, TITAN_MODEL, MAPPING_CASES);
}

fn assert_mapped(runtime: &Runtime, lookup: &Registry, model: &str, cases: &[(&str, &str)]) {
    for (source, expected) in cases {
        assert_eq!(
            map(runtime, lookup, model, source).expect("the fixture body maps"),
            json(expected),
            "{source}"
        );
    }
}

fn unsupported_embedding_model() {
    let lookup = registry(UNSUPPORTED_ENTRIES);
    let runtime = runtime();
    for (model, source) in UNSUPPORTED_CASES {
        let unknown =
            map(&runtime, &lookup, model, source).expect_err("the fixture body is refused");
        assert_eq!(unknown.to_string(), unsupported_message(model), "{model}");
    }
}
