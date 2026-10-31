use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_logging::s3_transport::option_value::OptionValue;
use betterlite_logging::s3_transport::options::PartitionGranularity;
use betterlite_logging::s3_v2::events::{Event, create_batch_element};
use betterlite_logging::s3_v2::parameters::resolve_s3_parameters;
use betterlite_logging::s3_v2::source::Source;
use betterlite_logging::s3_v2::{Feature, PartitionGranularityCache};
use betterlite_models::decisions::Json;
use chrono::{NaiveDate, NaiveDateTime};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::value::RawValue;

type Environment = BTreeMap<String, String>;

#[rstest]
#[case::malformed("not-an-integer")]
#[case::negative("-1")]
fn invalid_upload_default_uses_builtin_limit(#[case] configured: &str) {
    let environment = Environment::from([(
        "DEFAULT_S3_MAX_CONCURRENT_UPLOADS".to_owned(),
        configured.to_owned(),
    )]);
    let parameters = resolve_s3_parameters(
        &Source::<()>::default(),
        &Source::default(),
        None,
        &environment,
    );
    assert_eq!(parameters.max_concurrent_uploads, 16);
}

#[rstest]
#[case::missing("{}", 50_000_usize, Some(3_600_u64))]
#[case::coerced(
    r#"{"s3_max_queue_size":"42","s3_max_retry_age_seconds":"17"}"#,
    42_usize,
    Some(17_u64)
)]
#[case::invalid(
    r#"{"s3_max_queue_size":-1,"s3_max_retry_age_seconds":-4}"#,
    50_000_usize,
    Some(3_600_u64)
)]
#[case::zero(r#"{"s3_max_retry_age_seconds":0}"#, 50_000_usize, None)]
#[case::null_source_uses_constructor(
    r#"{"s3_max_retry_age_seconds":null}"#,
    50_000_usize,
    Some(3_600_u64)
)]
fn queue_and_retry_defaults(#[case] input: &str, #[case] queue: usize, #[case] age: Option<u64>) {
    let source: Source<()> = serde_json::from_str(input).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    assert_eq!(parameters.max_queue_size, queue);
    assert_eq!(
        parameters
            .max_retry_age_seconds
            .map(|duration| duration.as_secs()),
        age
    );
}

#[rstest]
#[case::null(r#"{"s3_max_retry_age_seconds":null}"#, None)]
#[case::zero(r#"{"s3_max_retry_age_seconds":0}"#, None)]
#[case::invalid(r#"{"s3_max_retry_age_seconds":-1}"#, Some(3_600_u64))]
fn constructor_retry(#[case] input: &str, #[case] expected: Option<u64>) {
    let constructor: Source<()> = serde_json::from_str(input).unwrap();
    let parameters = resolve_s3_parameters(
        &constructor,
        &Source::default(),
        None,
        &Environment::default(),
    );
    assert_eq!(
        parameters
            .max_retry_age_seconds
            .map(|duration| duration.as_secs()),
        expected
    );
}

#[test]
fn empty_audit_override_and_source_immutability() {
    let global: Source<()> = serde_json::from_str(r#"{"s3_bucket_name":"global"}"#).unwrap();
    let constructor = serde_json::from_str(r#"{"s3_bucket_name":"constructor"}"#).unwrap();
    let parameters = resolve_s3_parameters(
        &constructor,
        &global,
        Some(&Source::default()),
        &Environment::default(),
    );
    assert_eq!(
        parameters.connection.bucket_name.as_deref(),
        Some("constructor")
    );
    assert_eq!(global.bucket_name.as_deref(), Some("global"));
}

#[test]
fn invalid_source_bounds_use_correct_fallbacks() {
    let constructor: Source<()> = serde_json::from_str(
        r#"{"s3_max_concurrent_uploads":7,"s3_max_queue_size":100,"s3_max_retry_age_seconds":12}"#,
    )
    .unwrap();
    let source = serde_json::from_str(
        r#"{"s3_max_concurrent_uploads":-1,"s3_max_queue_size":-1,"s3_max_retry_age_seconds":-1}"#,
    )
    .unwrap();
    let parameters = resolve_s3_parameters(&constructor, &source, None, &Environment::default());
    assert_eq!(parameters.max_concurrent_uploads, 16);
    assert_eq!(parameters.max_queue_size, 100);
    assert_eq!(parameters.max_retry_age_seconds.unwrap().as_secs(), 12);
}

#[test]
fn partition_cache_invalidates_on_raw_value_and_environment() {
    let mut cache = PartitionGranularityCache::default();
    let mut environment = Environment::default();
    assert_eq!(cache.resolve(None, &environment), PartitionGranularity::Day);
    environment.insert("S3_PARTITION_GRANULARITY".to_owned(), "hour".to_owned());
    assert_eq!(
        cache.resolve(None, &environment),
        PartitionGranularity::Hour
    );
    assert_eq!(
        cache.resolve(None, &environment),
        PartitionGranularity::Hour
    );
    let configured = OptionValue::Text("invalid".to_owned());
    assert_eq!(
        cache.resolve(Some(&configured), &environment),
        PartitionGranularity::Day
    );
}

#[rstest]
#[case::day(
    "logs/team/key/2026-01-02/time-03-04-05-000000_request.json",
    true,
    "day"
)]
#[case::hour(
    "logs/team/key/2026-01-02/03/time-03-04-05-000000_request.json",
    true,
    "hour"
)]
#[case::mismatch("somewhere/else.json", true, "hour")]
#[case::disabled(
    "logs/team/key/2026-01-02/time-03-04-05-000000_request.json",
    false,
    "hour"
)]
fn cold_storage_keys(#[case] cold_key: &str, #[case] enabled: bool, #[case] expected: &str) {
    let source: Source<()> = serde_json::from_str(r#"{"s3_path":"logs","s3_use_team_prefix":true,"s3_use_key_prefix":true,"s3_partition_granularity":"hour"}"#).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    let encoded_key = serde_json::to_string(cold_key).unwrap();
    let payload: Box<RawValue> = serde_json::from_str(&format!(r#"{{"id":"request","metadata":{{"user_api_key_team_alias":"team","user_api_key_alias":"key","cold_storage_object_key":{encoded_key}}}}}"#)).unwrap();
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: enabled,
        },
        &mut PartitionGranularityCache::default(),
        &Environment::default(),
    )
    .unwrap()
    .unwrap();
    let segment = if expected == "hour" {
        "2026-01-02/03"
    } else {
        "2026-01-02"
    };
    assert_eq!(
        output.s3_object_key,
        format!("logs/team/key/{segment}/time-03-04-05-000000_request.json")
    );
    assert_eq!(
        output.s3_object_download_filename,
        "time-2026-01-02T03-04-05-000000_request.json"
    );
    assert!(output.body.is_none());
}

#[test]
fn cold_storage_key_numeric_value_is_a_mismatch() {
    let source: Source<()> = serde_json::from_str(r#"{"s3_path":"logs","s3_use_team_prefix":true,"s3_use_key_prefix":true,"s3_partition_granularity":"hour"}"#).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    let payload: Box<RawValue> = serde_json::from_str(r#"{"id":"request","metadata":{"user_api_key_team_alias":"team","user_api_key_alias":"key","cold_storage_object_key":123}}"#).unwrap();
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: true,
        },
        &mut PartitionGranularityCache::default(),
        &Environment::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        output.s3_object_key,
        "logs/team/key/2026-01-02/03/time-03-04-05-000000_request.json"
    );
}

#[derive(Deserialize)]
struct PayloadView {
    messages: Vec<Message>,
    response: Option<String>,
    metadata: BTreeMap<String, String>,
}
#[derive(Deserialize)]
struct Message {
    content: Vec<TextPart>,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct TextPart {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[test]
fn strip_base64_and_prompt_projection_are_immutable() {
    let source: Source<()> = serde_json::from_str(r#"{"s3_strip_base64_files":true}"#).unwrap();
    let environment = Environment::from([("S3_LOG_PROMPTS_ONLY".to_owned(), "true".to_owned())]);
    let parameters = resolve_s3_parameters(&Source::default(), &source, None, &environment);
    let payload: Box<RawValue> = serde_json::from_str(r#"{"id":"request","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}},{"type":"text","text":"before data:image/png;base64,AAAA! after"},{"file":{}}]}],"response":{"text":"ok"},"metadata":{"arbitrary":"kept"}}"#).unwrap();
    let before = payload.get().to_owned();
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: false,
        },
        &mut PartitionGranularityCache::default(),
        &environment,
    )
    .unwrap()
    .unwrap();
    let projected: PayloadView = serde_json::from_str(output.payload.get()).unwrap();
    assert_eq!(payload.get(), before);
    assert_eq!(
        projected.messages.first().unwrap().content,
        vec![TextPart {
            kind: "text".to_owned(),
            text: "before [BASE64_REDACTED]! after".to_owned()
        }]
    );
    assert!(projected.response.is_none());
    assert_eq!(
        projected.metadata.get("arbitrary").map(String::as_str),
        Some("kept")
    );
}

#[rstest]
#[case::default_depth_truncates(None, Json::Text("[MAX_DEPTH_REACHED]".to_owned()))]
#[case::configured_depth_preserves(Some("20"), Json::Text("[BASE64_REDACTED]".to_owned()))]
#[case::malformed_depth_falls_back_to_default(
    Some("not-a-number"),
    Json::Text("[MAX_DEPTH_REACHED]".to_owned())
)]
#[case::negative_depth_falls_back_to_default(
    Some("-1"),
    Json::Text("[MAX_DEPTH_REACHED]".to_owned())
)]
fn strip_base64_respects_configured_recursion_depth(
    #[case] configured: Option<&str>,
    #[case] expected_leaf: Json,
) {
    let source: Source<()> = serde_json::from_str(r#"{"s3_strip_base64_files":true}"#).unwrap();
    let mut environment = Environment::default();
    if let Some(value) = configured {
        environment.insert(
            "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER".to_owned(),
            value.to_owned(),
        );
    }
    let parameters = resolve_s3_parameters(&Source::default(), &source, None, &environment);
    let nested = r#"{"inner":{"inner":{"inner":{"inner":{"inner":{"inner":{"inner":{"inner":{"inner":{"inner":{"inner":"data:image/png;base64,AAAA"}}}}}}}}}}}"#;
    let payload: Box<RawValue> = serde_json::from_str(&format!(
        r#"{{"id":"request","messages":[{{"role":"user","content":{nested}}}]}}"#
    ))
    .unwrap();
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: false,
        },
        &mut PartitionGranularityCache::default(),
        &environment,
    )
    .unwrap()
    .unwrap();
    let projected: Json = serde_json::from_str(output.payload.get()).unwrap();
    let Json::Object(fields) = &projected else {
        panic!("expected a JSON object");
    };
    let Some(Json::List(messages)) = fields.get("messages") else {
        panic!("expected a messages list");
    };
    let Some(Json::Object(message)) = messages.first() else {
        panic!("expected a message object");
    };
    let mut leaf = message.get("content").unwrap();
    for _ in 0..11_u8 {
        let Json::Object(wrapper) = leaf else {
            panic!("expected a nested object");
        };
        leaf = wrapper.get("inner").unwrap();
    }
    assert_eq!(leaf, &expected_leaf);
}

#[test]
fn strip_base64_drops_a_content_item_with_a_null_file_key() {
    let source: Source<()> = serde_json::from_str(r#"{"s3_strip_base64_files":true}"#).unwrap();
    let payload: Box<RawValue> = serde_json::from_str(r#"{"id":"request","messages":[{"role":"user","content":[{"type":"text","text":"keep"},{"file":null}]}]}"#).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: false,
        },
        &mut PartitionGranularityCache::default(),
        &Environment::default(),
    )
    .unwrap()
    .unwrap();
    let projected: Json = serde_json::from_str(output.payload.get()).unwrap();
    let Json::Object(fields) = &projected else {
        panic!("expected a JSON object");
    };
    let Some(Json::List(messages)) = fields.get("messages") else {
        panic!("expected a messages list");
    };
    let Some(Json::Object(message)) = messages.first() else {
        panic!("expected a message object");
    };
    let Some(Json::List(content)) = message.get("content") else {
        panic!("expected a content list");
    };
    assert_eq!(content.len(), 1);
}

#[test]
fn strip_base64_preserves_null_messages_and_response() {
    let source: Source<()> = serde_json::from_str(r#"{"s3_strip_base64_files":true}"#).unwrap();
    let payload: Box<RawValue> =
        serde_json::from_str(r#"{"id":"request","messages":null,"response":null}"#).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: false,
        },
        &mut PartitionGranularityCache::default(),
        &Environment::default(),
    )
    .unwrap()
    .unwrap();
    let projected: Json = serde_json::from_str(output.payload.get()).unwrap();
    let Json::Object(fields) = &projected else {
        panic!("expected a JSON object");
    };
    assert_eq!(fields.get("messages"), Some(&Json::Null));
    assert_eq!(fields.get("response"), Some(&Json::Null));
}

#[test]
fn prompts_only_alone_leaves_a_contentless_message_untouched() {
    let source: Source<()> = serde_json::from_str("{}").unwrap();
    let environment = Environment::from([("S3_LOG_PROMPTS_ONLY".to_owned(), "true".to_owned())]);
    let payload: Box<RawValue> = serde_json::from_str(
        r#"{"id":"request","messages":[{"role":"user"}],"response":{"text":"ok"}}"#,
    )
    .unwrap();
    let parameters = resolve_s3_parameters(&Source::default(), &source, None, &environment);
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: false,
        },
        &mut PartitionGranularityCache::default(),
        &environment,
    )
    .unwrap()
    .unwrap();
    let projected: Json = serde_json::from_str(output.payload.get()).unwrap();
    let Json::Object(fields) = &projected else {
        panic!("expected a JSON object");
    };
    let Some(Json::List(messages)) = fields.get("messages") else {
        panic!("expected a messages list");
    };
    let Some(Json::Object(message)) = messages.first() else {
        panic!("expected a message object");
    };
    assert!(!message.contains_key("content"));
    assert_eq!(fields.get("response"), Some(&Json::Null));
}

#[test]
fn strip_base64_appends_a_null_content_key_to_a_contentless_message() {
    let source: Source<()> = serde_json::from_str(r#"{"s3_strip_base64_files":true}"#).unwrap();
    let payload: Box<RawValue> =
        serde_json::from_str(r#"{"id":"request","messages":[{"role":"user"}]}"#).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    let time = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 5)
        .unwrap();
    let output = create_batch_element(
        &parameters,
        &Event {
            start_time: time,
            payload: Some(&payload),
            cold_storage_enabled: false,
        },
        &mut PartitionGranularityCache::default(),
        &Environment::default(),
    )
    .unwrap()
    .unwrap();
    let projected: Json = serde_json::from_str(output.payload.get()).unwrap();
    let Json::Object(fields) = &projected else {
        panic!("expected a JSON object");
    };
    let Some(Json::List(messages)) = fields.get("messages") else {
        panic!("expected a messages list");
    };
    let Some(Json::Object(message)) = messages.first() else {
        panic!("expected a message object");
    };
    assert_eq!(message.get("content"), Some(&Json::Null));
    assert_eq!(message.keys().last().map(String::as_str), Some("content"));
}

#[test]
fn constructor_flags_and_encryption_precedence() {
    let constructor: Source<()> = serde_json::from_str(r#"{"s3_use_team_prefix":true,"s3_log_prompts_only":false,"s3_partition_granularity":"hour","s3_aws_session_name":"session"}"#).unwrap();
    let source = serde_json::from_str(r#"{"s3_use_team_prefix":false,"s3_use_key_prefix":"false","s3_sse_kms_key_id":"kms","s3_log_prompts_only":true}"#).unwrap();
    let parameters = resolve_s3_parameters(&constructor, &source, None, &Environment::default());
    assert!(parameters.features.contains(&Feature::TeamPrefix));
    assert!(parameters.features.contains(&Feature::KeyPrefix));
    assert_eq!(parameters.encryption.unwrap().algorithm, "aws:kms");
    assert_eq!(
        parameters
            .connection
            .credentials
            .aws_session_name
            .as_deref(),
        Some("session")
    );
    assert_eq!(
        parameters.connection.log_prompts_only,
        constructor.log_prompts_only
    );
    assert_eq!(
        parameters.connection.partition_granularity,
        constructor.partition_granularity
    );
}

#[test]
fn replays_the_s3_v2_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/s3_v2");
    let results: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            let (source, payload): (Source<()>, Option<Box<RawValue>>) =
                serde_json::from_slice(&bytes).unwrap();
            let parameters =
                resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
            let result = create_batch_element(
                &parameters,
                &Event {
                    start_time: NaiveDateTime::default(),
                    payload: payload.as_ref(),
                    cold_storage_enabled: true,
                },
                &mut PartitionGranularityCache::default(),
                &Environment::default(),
            );
            if path.ends_with("missing-id.json") {
                assert!(result.is_err());
            } else {
                let _element = result.unwrap();
            }
        })
        .collect();
    assert!(results.len() >= 4);
}

#[test]
fn credentials_are_redacted_and_markers_are_preserved() {
    let source: Source<()> = serde_json::from_str(
        r#"{"s3_aws_secret_access_key":"os.environ/SECRET","s3_aws_session_token":"token"}"#,
    )
    .unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    assert_eq!(
        parameters
            .connection
            .credentials
            .aws_secret_access_key
            .as_ref()
            .unwrap()
            .expose_secret(),
        "os.environ/SECRET"
    );
    assert!(!format!("{parameters:?}").contains("os.environ/SECRET"));
    assert!(!format!("{source:?}").contains("\"token\""));
}

#[rstest]
#[case::zero_float("0.0", false)]
#[case::nonzero_float("0.5", true)]
#[case::empty_array("[]", false)]
#[case::array("[null]", true)]
#[case::empty_object("{}", false)]
#[case::object("{\"key\":null}", true)]
#[case::nonempty_string("\"false\"", true)]
#[case::null("null", false)]
fn legacy_flag_truthiness(#[case] input: &str, #[case] expected: bool) {
    let source: Source<()> =
        serde_json::from_str(&format!("{{\"s3_use_team_prefix\":{input}}}")).unwrap();
    let parameters =
        resolve_s3_parameters(&Source::default(), &source, None, &Environment::default());
    assert_eq!(parameters.features.contains(&Feature::TeamPrefix), expected);
}

#[test]
fn no_event_on_missing_payload() {
    let environment = Environment::default();
    let parameters = resolve_s3_parameters(
        &Source::<()>::default(),
        &Source::default(),
        None,
        &environment,
    );
    let time = NaiveDateTime::default();
    assert!(
        create_batch_element::<(), Box<RawValue>>(
            &parameters,
            &Event {
                start_time: time,
                payload: None,
                cold_storage_enabled: false
            },
            &mut PartitionGranularityCache::default(),
            &environment
        )
        .unwrap()
        .is_none()
    );
}
