use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::ready;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use super::chat_body::text_completion_to_chat;
use super::error::BatchRecordsError;
use super::families::{BatchFamilies, Params};
use super::jsonl::{BatchRecordContext, transform_openai_jsonl};
use super::record::{LINE_BOUNDARIES, OpenAiBatchRecord, decode_batch_input_line};
use super::upload::{CreateFileRequest, FileUploadHandoff, transform_create_file_request};
use crate::aws_settings::sources::Credentials;
use crate::batch_embeddings::classify::BatchRecordKind;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params as RequestParams, RequestValue};
use crate::signing::target::RegionInputs;

const FIXED_UUID: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001);
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const OTHER_SECRET_KEY: &str = "OTHERsEcReTkEyF0rIsOlAtIoNtEsTs00000000000000";
const ACCESS_KEY: &str = "AKIDEXAMPLE";
const OTHER_ACCESS_KEY: &str = "AKIDOTHEREXAMPLE";
const GATEWAY_MODEL: &str = "openai.gpt-oss-20b-1:0";

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

fn value(json: &Json) -> serde_json::Value {
    serde_json::to_value(json).expect("the fixture value renders")
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("the fixture params parse")
}

fn text_params(entries: &[(&str, &str)]) -> RequestParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), RequestValue::Text((*value).to_owned())))
        .collect()
}

#[derive(Debug, Clone)]
struct Call {
    model: String,
    messages: Vec<Json>,
    optional_params: Params,
}

#[derive(Default)]
struct Families {
    nova_calls: Mutex<Vec<Call>>,
    anthropic_params_calls: Mutex<Vec<(String, Params)>>,
    anthropic_body_calls: Mutex<Vec<Call>>,
    responses_calls: Mutex<Vec<Json>>,
    nova_body: Option<Json>,
    anthropic_params: Option<Params>,
    anthropic_body: Option<Json>,
    responses_body: Option<Json>,
    batch: bool,
}

impl Families {
    fn nova() -> Self {
        Self {
            nova_body: Some(json(r#"{"messages":[],"inferenceConfig":{}}"#)),
            ..Self::default()
        }
    }

    fn batch() -> Self {
        Self {
            batch: true,
            nova_body: Some(json(r#"{"messages":[],"inferenceConfig":{}}"#)),
            ..Self::default()
        }
    }

    fn nova_calls(&self) -> Vec<Call> {
        self.nova_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }

    fn anthropic_params_calls(&self) -> Vec<(String, Params)> {
        self.anthropic_params_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }

    fn anthropic_body_calls(&self) -> Vec<Call> {
        self.anthropic_body_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }

    fn responses_calls(&self) -> Vec<Json> {
        self.responses_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }
}

#[async_trait::async_trait]
impl BatchFamilies for Families {
    async fn anthropic_params(
        &self,
        mapping_model: &str,
        optional_params: &Params,
    ) -> Result<Params, super::families::Failure> {
        self.anthropic_params_calls
            .lock()
            .expect("no panic held the lock")
            .push((mapping_model.to_owned(), optional_params.clone()));
        Ok(self.anthropic_params.clone().unwrap_or_default())
    }

    async fn anthropic_body(
        &self,
        model: &str,
        messages: &[Json],
        optional_params: &Params,
    ) -> Result<Json, super::families::Failure> {
        self.anthropic_body_calls
            .lock()
            .expect("no panic held the lock")
            .push(Call {
                model: model.to_owned(),
                messages: messages.to_vec(),
                optional_params: optional_params.clone(),
            });
        Ok(self.anthropic_body.clone().unwrap_or(Json::Null))
    }

    async fn nova_body(
        &self,
        model: &str,
        messages: &[Json],
        optional_params: &Params,
    ) -> Result<Json, super::families::Failure> {
        self.nova_calls
            .lock()
            .expect("no panic held the lock")
            .push(Call {
                model: model.to_owned(),
                messages: messages.to_vec(),
                optional_params: optional_params.clone(),
            });
        Ok(self.nova_body.clone().unwrap_or(Json::Null))
    }

    async fn responses_to_chat(&self, body: &Json) -> Result<Json, super::families::Failure> {
        self.responses_calls
            .lock()
            .expect("no panic held the lock")
            .push(body.clone());
        Ok(self.responses_body.clone().unwrap_or(Json::Null))
    }

    fn is_batch_jsonl(
        &self,
        _purpose: Option<&str>,
        _content_type: Option<&str>,
        _body: Option<&[u8]>,
    ) -> bool {
        self.batch
    }
}

struct StubLookup;

impl ModelInfoLookup for StubLookup {
    type Error = Infallible;

    fn bedrock_model_info(
        &self,
        _model: &str,
    ) -> impl std::future::Future<Output = Result<ModelInfo, Infallible>> + Send {
        ready(Ok(ModelInfo {
            fields: IndexMap::new(),
        }))
    }
}

struct Sources {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Sources {
    fn load() -> Self {
        Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
        }
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: std::sync::Arc::new(NoOidc),
        azure_tokens: std::sync::Arc::new(NoOidc),
        clock: std::sync::Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn record(source: &str) -> OpenAiBatchRecord {
    decode_batch_input_line(source.as_bytes())
        .expect("the record line decodes")
        .expect("the record line is not blank")
}

fn transform(
    families: &Families,
    source: &str,
    target_model: &str,
) -> Result<Vec<Json>, BatchRecordsError> {
    let record = record(source);
    let aliases = BTreeMap::new();
    transform_with(families, &[record], target_model, &aliases)
}

fn transform_with(
    families: &Families,
    records: &[OpenAiBatchRecord],
    target_model: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<Vec<Json>, BatchRecordsError> {
    let sources = Sources::load();
    let resolution = sources.resolution();
    let lookup = StubLookup;
    let context = BatchRecordContext {
        aliases,
        families,
        lookup: &lookup,
        resolution: &resolution,
    };
    runtime()
        .block_on(transform_openai_jsonl(records, target_model, &context))
        .map(|records| {
            records
                .iter()
                .map(super::record::BedrockBatchRecord::to_json)
                .collect()
        })
}

fn error_of(result: Result<Vec<Json>, BatchRecordsError>) -> BatchRecordsError {
    result.expect_err("the fixture input is refused")
}

#[rstest]
#[case::chat_batch_record(chat_batch_record)]
#[case::text_completion_to_chat(text_completion_to_chat_case)]
#[case::responses_to_chat(responses_to_chat)]
#[case::embedding_batch_record(embedding_batch_record)]
#[case::alias_and_target_model(alias_and_target_model)]
#[case::default_record_id(default_record_id)]
#[case::jsonl_blank_lines(jsonl_blank_lines)]
#[case::malformed_jsonl_line(malformed_jsonl_line)]
#[case::nova_batch_record(nova_batch_record)]
#[case::upload_signing_handoff(upload_signing_handoff)]
fn batch_records_cases(#[case] check: fn()) {
    check();
}

fn chat_batch_record() {
    let families = Families::default();
    let records = transform(
        &families,
        r#"{"custom_id":"a","url":"/v1/chat/completions","body":{"model":"openai.gpt-oss-20b-1:0","messages":[{"role":"developer","content":"be nice"},{"content":"hi","role":"user"}],"max_tokens":16}}"#,
        "",
    )
    .expect("the chat record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({
            "recordId": "a",
            "modelInput": {
                "messages": [
                    {"role": "system", "content": "be nice"},
                    {"content": "hi", "role": "user"}
                ],
                "max_tokens": 16
            }
        })
    );
    // a passthrough provider names no family, so nothing was mapped for it
    assert!(families.nova_calls().is_empty());
    assert!(families.anthropic_params_calls().is_empty());
    assert!(families.responses_calls().is_empty());
}

fn text_completion_to_chat_case() {
    let families = Families::default();
    let records = transform(
        &families,
        r#"{"custom_id":"c","url":"/v1/completions","body":{"model":"qwen.qwen3-32b-v1:0","prompt":"hello"}}"#,
        "",
    )
    .expect("the completion record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({
            "recordId": "c",
            "modelInput": {"messages": [{"content": "hello", "role": "user"}]}
        })
    );
    let error = error_of(transform(
        &families,
        r#"{"url":"/v1/completions","body":{"model":"qwen.qwen3-32b-v1:0"}}"#,
        "",
    ));
    assert_eq!(
        error.to_string(),
        "Batch record for /v1/completions is missing required `prompt` field: model=qwen.qwen3-32b-v1:0"
    );
    let null_prompt = text_completion_to_chat(&json(r#"{"model":"m","prompt":null}"#))
        .expect_err("a null prompt is refused");
    assert!(
        null_prompt
            .to_string()
            .contains("missing required `prompt`")
    );
    // an empty prompt is a prompt, so the message list is empty and the conversion succeeds
    let empty =
        text_completion_to_chat(&json(r#"{"model":"m","prompt":""}"#)).expect("empty prompt maps");
    assert_eq!(value(&empty)["messages"], serde_json::json!([]));
}

fn responses_to_chat() {
    let families = Families {
        responses_body: Some(json(
            r#"{"messages":[{"role":"user","content":"hi"}],"max_output_tokens":5}"#,
        )),
        ..Families::default()
    };
    let source = json(
        r#"{"custom_id":"r","url":"/v1/responses","body":{"model":"qwen.qwen3-32b-v1:0","input":"hi","max_output_tokens":5}}"#,
    );
    let records = transform(
        &families,
        &serde_json::to_string(&value(&source)).expect("renders"),
        "",
    )
    .expect("the responses record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({
            "recordId": "r",
            "modelInput": {
                "messages": [{"content": "hi", "role": "user"}],
                "max_output_tokens": 5
            }
        })
    );
    assert_eq!(families.responses_calls(), vec![source]);
}

fn embedding_batch_record() {
    let families = Families::default();
    let records = transform(
        &families,
        r#"{"custom_id":"e","url":"/v1/embeddings","body":{"model":"amazon.titan-embed-text-v2:0","input":"hello"}}"#,
        "",
    )
    .expect("the embedding record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({"recordId": "e", "modelInput": {"inputText": "hello"}})
    );
    // the chat families never see an embedding record
    assert!(families.nova_calls().is_empty());
}

fn alias_and_target_model() {
    let families = Families::nova();
    let aliases = BTreeMap::from([(
        String::from("my-alias"),
        String::from("amazon.nova-pro-v1:0"),
    )]);
    let aliased = record(
        r#"{"custom_id":"a","url":"/v1/chat/completions","body":{"model":"my-alias","messages":[{"content":"hi","role":"user"}]}}"#,
    );
    transform_with(&families, &[aliased], "", &aliases).expect("the aliased record transforms");
    let calls = families.nova_calls();
    assert_eq!(calls[0].model, "amazon.nova-pro-v1:0");

    let families = Families::nova();
    let unnamed = record(
        r#"{"custom_id":"b","url":"/v1/chat/completions","body":{"model":"my-custom","messages":[{"content":"hi","role":"user"}]}}"#,
    );
    transform_with(
        &families,
        &[unnamed],
        "amazon.nova-pro-v1:0",
        &BTreeMap::new(),
    )
    .expect("the target model resolves the record");
    assert_eq!(families.nova_calls()[0].model, "amazon.nova-pro-v1:0");

    // a record that names its own provider keeps that model even when a target model exists
    let families = Families::default();
    let named = record(
        r#"{"custom_id":"c","url":"/v1/chat/completions","body":{"model":"openai.gpt-oss-20b-1:0","messages":[{"content":"hi","role":"user"}]}}"#,
    );
    transform_with(
        &families,
        &[named],
        "amazon.nova-pro-v1:0",
        &BTreeMap::new(),
    )
    .expect("the named record transforms");
    assert!(families.nova_calls().is_empty());
}

fn default_record_id() {
    let families = Families::default();
    let sources = [
        "/v1/chat/completions",
        "/v1/chat/completions",
        "/v1/chat/completions",
    ];
    let bodies = [
        r#"{"body":{"model":"openai.gpt-oss-20b-1:0","messages":[]}}"#,
        r#"{"custom_id":null,"body":{"model":"openai.gpt-oss-20b-1:0","messages":[]}}"#,
        r#"{"custom_id":"z","body":{"model":"openai.gpt-oss-20b-1:0","messages":[]}}"#,
    ];
    let records: Vec<OpenAiBatchRecord> = sources
        .iter()
        .zip(bodies)
        .map(|(url, body)| {
            let line = format!(r#"{{"url":"{url}",{body}"#);
            record(&line)
        })
        .collect();
    let transformed =
        transform_with(&families, &records, "", &BTreeMap::new()).expect("the records transform");
    assert_eq!(
        value(&transformed[0])["recordId"],
        value(&json(r#""CALL0000000""#))
    );
    assert_eq!(value(&transformed[1])["recordId"], serde_json::Value::Null);
    assert_eq!(value(&transformed[2])["recordId"], value(&json(r#""z""#)));
    // the width pads and never truncates
    assert_eq!(records[0].record_id(9), json(r#""CALL0000009""#));
    assert_eq!(records[0].record_id(10), json(r#""CALL0000010""#));
    assert_eq!(records[0].record_id(12_345_678), json(r#""CALL12345678""#));
}

fn jsonl_blank_lines() {
    let families = Families::batch();
    let file = FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(
            "\n  \u{1f}\n{\"custom_id\":\"a\",\"body\":{\"model\":\"openai.gpt-oss-20b-1:0\",\"messages\":[{\"content\":\"hi\",\"role\":\"user\"}]}}\r\n\u{2028}\n"
                .as_bytes()
                .to_vec(),
        ),
        content_type: Some(String::from("application/jsonl")),
    };
    let (handoff, _litellm_params) = run_upload(
        &families,
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    let handoff = handoff.expect("the batch upload signs");
    assert_eq!(
        handoff.data,
        "{\"recordId\": \"a\", \"modelInput\": {\"messages\": [{\"content\": \"hi\", \"role\": \"user\"}]}}"
    );
    let (empty, _params) = run_upload(
        &Families::batch(),
        &FileData::Direct(FileBody::Bytes(b"\n\n".to_vec())),
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert_eq!(empty.expect("the empty batch upload signs").data, "");
    assert_eq!(LINE_BOUNDARIES.len(), 10);
}

fn malformed_jsonl_line() {
    let families = Families::batch();
    let malformed = FileData::Direct(FileBody::Bytes(b"{\"custom_id\":".to_vec()));
    let (result, _params) = run_upload(
        &families,
        &malformed,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert!(matches!(
        result.expect_err("the malformed line is refused"),
        BatchRecordsError::Json(_)
    ));
    let invalid_utf8 = FileData::Direct(FileBody::Bytes(vec![0xff, 0xfe]));
    let (result, _params) = run_upload(
        &families,
        &invalid_utf8,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert!(matches!(
        result.expect_err("the invalid UTF-8 line is refused"),
        BatchRecordsError::Utf8(_)
    ));
    // a record whose body is not an object never reaches a family
    let family = FileData::Direct(FileBody::Bytes(
        b"{\"custom_id\":\"a\",\"body\":[1]}".to_vec(),
    ));
    let (result, _params) = run_upload(
        &Families::batch(),
        &family,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert!(matches!(
        result.expect_err("the array body is refused"),
        BatchRecordsError::RecordShape
    ));
    // a non-batch file keeps its bytes as UTF-8 text
    let plain = FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(b"plain text".to_vec()),
        content_type: Some(String::from("text/plain")),
    };
    let (result, _params) = run_upload(
        &Families::default(),
        &plain,
        None,
        "",
        &mut booking_params(),
    );
    let handoff = result.expect("the plain upload signs");
    assert_eq!(handoff.data, "plain text");
    let bare = FileData::Direct(FileBody::BareString(String::from("plain text")));
    let (result, _params) =
        run_upload(&Families::default(), &bare, None, "", &mut booking_params());
    assert!(matches!(
        result.expect_err("the bare string is refused"),
        BatchRecordsError::Media(_)
    ));
    let (missing, _params) = run_upload_missing_file(&Families::default());
    assert_eq!(missing.to_string(), "file is required");
}

fn nova_batch_record() {
    let families = Families::nova();
    let records = transform(
        &families,
        r#"{"custom_id":"n","url":"/v1/chat/completions","body":{"model":"amazon.nova-pro-v1:0","messages":[{"role":"developer","content":"be nice"},{"content":"hi","role":"user"}],"max_tokens":16,"top_k":5}}"#,
        "",
    )
    .expect("the nova record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({"recordId": "n", "modelInput": {"messages": [], "inferenceConfig": {}}})
    );
    let calls = families.nova_calls();
    assert_eq!(calls[0].model, "amazon.nova-pro-v1:0");
    // the nova branch hands every non-message field to the mapper, with the developer message projected
    assert_eq!(
        value(&Json::List(calls[0].messages.clone())),
        serde_json::json!([
            {"role": "system", "content": "be nice"},
            {"content": "hi", "role": "user"}
        ])
    );
    assert_eq!(
        calls[0].optional_params,
        params(r#"{"max_tokens":16,"top_k":5}"#)
    );
    assert!(families.anthropic_params_calls().is_empty());
}

fn upload_signing_handoff() {
    let families = Families::batch();
    let file = batch_file();
    let mut litellm_params = booking_params();
    litellm_params.insert(
        String::from("s3_encryption_key_id"),
        RequestValue::Text(String::from("key-1")),
    );
    let (handoff, after) = run_upload(
        &families,
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut litellm_params,
    );
    let handoff = handoff.expect("the batch upload signs");
    assert_eq!(handoff.method, "PUT");
    assert!(
        handoff
            .url
            .starts_with("https://s3.us-east-1.amazonaws.com/my-bucket/")
    );
    assert!(
        handoff
            .url
            .ends_with(&format!("-{}.jsonl", FIXED_UUID.hyphenated()))
    );
    let headers = &handoff.headers;
    assert!(headers.contains(&(
        String::from("Content-Type"),
        String::from("application/json")
    )));
    assert!(headers.contains(&(String::from("Content-Language"), String::from("en"))));
    assert!(headers.contains(&(
        String::from("Cache-Control"),
        String::from("private, immutable, max-age=31536000, s-maxage=0")
    )));
    assert!(headers.contains(&(
        String::from("x-amz-server-side-encryption"),
        String::from("aws:kms")
    )));
    assert!(headers.contains(&(
        String::from("x-amz-server-side-encryption-aws-kms-key-id"),
        String::from("key-1")
    )));
    assert!(headers.contains(&(
        String::from("Content-Length"),
        handoff.data.len().to_string()
    )));
    let authorization = headers
        .iter()
        .find(|(name, _value)| name == "Authorization")
        .expect("the upload is signed")
        .1
        .clone();
    assert!(authorization.contains(ACCESS_KEY));
    assert!(!authorization.contains(SECRET_KEY));
    assert!(headers.iter().any(|(name, _value)| name == "X-Amz-Date"));
    assert!(
        headers
            .iter()
            .any(|(name, _value)| name == "X-Amz-Content-SHA256")
    );
    assert_eq!(
        after.get("upload_url"),
        Some(&RequestValue::Text(handoff.url.clone()))
    );
    assert_eq!(
        after.get("_s3_upload_content_length"),
        Some(&RequestValue::Raw(handoff.data.len().to_string()))
    );
    // the same batch record with a target model instead of a record model takes the litellm model
    let mut params_with_model = booking_params();
    params_with_model.insert(
        String::from("model"),
        RequestValue::Text(String::from(GATEWAY_MODEL)),
    );
    let (handoff, _after) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut params_with_model,
    );
    assert!(
        handoff
            .expect("the target model upload signs")
            .data
            .contains("\"messages\"")
    );
}

#[test]
fn batch_records_credential_identity_isolation() {
    let file = batch_file();
    let (first, first_params) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    let first = first.expect("the first identity uploads");
    let mut other = booking_params();
    other.insert(
        String::from("s3_access_key_id"),
        RequestValue::Text(String::from(OTHER_ACCESS_KEY)),
    );
    other.insert(
        String::from("s3_secret_access_key"),
        RequestValue::Secret(SecretString::from(OTHER_SECRET_KEY)),
    );
    let (second, second_params) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut other,
    );
    let second = second.expect("the second identity uploads");
    let authorization = |handoff: &FileUploadHandoff| {
        handoff
            .headers
            .iter()
            .find(|(name, _value)| name == "Authorization")
            .expect("the upload is signed")
            .1
            .clone()
    };
    let first_authorization = authorization(&first);
    let second_authorization = authorization(&second);
    assert_ne!(first_authorization, second_authorization);
    assert!(first_authorization.contains(ACCESS_KEY));
    assert!(!first_authorization.contains(OTHER_ACCESS_KEY));
    assert!(second_authorization.contains(OTHER_ACCESS_KEY));
    assert!(!second_authorization.contains(ACCESS_KEY));
    // the same object with the same id is the same URL: nothing about identity leaks into it
    assert_eq!(first.url, second.url);
    assert_eq!(
        first_params.get("s3_access_key_id"),
        Some(&RequestValue::Text(String::from(ACCESS_KEY)))
    );
    assert_eq!(
        second_params.get("s3_access_key_id"),
        Some(&RequestValue::Text(String::from(OTHER_ACCESS_KEY)))
    );
    assert!(!format!("{:?}", first_params).contains(SECRET_KEY));
    assert!(!format!("{:?}", second_params).contains(OTHER_SECRET_KEY));
}

#[test]
fn batch_records_secret_safe_failure() {
    let file = batch_file();
    let mut without_bucket = RequestParams::new();
    without_bucket.insert(
        String::from("s3_access_key_id"),
        RequestValue::Text(String::from(ACCESS_KEY)),
    );
    without_bucket.insert(
        String::from("s3_secret_access_key"),
        RequestValue::Secret(SecretString::from(SECRET_KEY)),
    );
    without_bucket.insert(
        String::from("model"),
        RequestValue::Text(String::from(GATEWAY_MODEL)),
    );
    let (error, params) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut without_bucket,
    );
    let error = error.expect_err("the upload without a bucket is refused");
    assert!(matches!(error, BatchRecordsError::Signing(_)));
    assert!(!error.to_string().contains(SECRET_KEY));
    assert!(!format!("{error:?}").contains(SECRET_KEY));
    assert!(!format!("{params:?}").contains(SECRET_KEY));
    let (handoff, _after) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    let handoff = handoff.expect("the upload signs");
    // Debug names the headers and the data length, never a signature or a key
    let rendered = format!("{handoff:?}");
    assert!(!rendered.contains(SECRET_KEY));
    assert!(!rendered.contains("Authorization"));
    assert!(rendered.contains("header_names"));
    assert!(rendered.contains("data_length"));
}

fn batch_file() -> FileData {
    FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(
            b"{\"custom_id\":\"a\",\"body\":{\"model\":\"openai.gpt-oss-20b-1:0\",\"messages\":[{\"content\":\"hi\",\"role\":\"user\"}]}}"
                .to_vec(),
        ),
        content_type: Some(String::from("application/jsonl")),
    }
}

fn booking_params() -> RequestParams {
    let mut params = text_params(&[
        ("s3_bucket_name", "my-bucket"),
        ("s3_region_name", "us-east-1"),
        ("s3_access_key_id", ACCESS_KEY),
    ]);
    params.insert(
        String::from("s3_secret_access_key"),
        RequestValue::Secret(SecretString::from(SECRET_KEY)),
    );
    params
}

fn resolve_credentials(
    _settings: &AwsSettings,
    _region: &str,
) -> std::future::Ready<Result<Credentials, CredentialError>> {
    ready(Ok(Credentials {
        access_key_id: SecretString::from(ACCESS_KEY),
        secret_access_key: SecretString::from(SECRET_KEY),
        session_token: None,
    }))
}

fn run_upload(
    families: &Families,
    file: &FileData,
    purpose: Option<CreatePurpose>,
    model: &str,
    litellm_params: &mut RequestParams,
) -> (Result<FileUploadHandoff, BatchRecordsError>, RequestParams) {
    let result = upload_request(families, Some(file), purpose, model, litellm_params);
    (result, litellm_params.clone())
}

fn run_upload_missing_file(families: &Families) -> (BatchRecordsError, RequestParams) {
    let mut litellm_params = booking_params();
    let result = upload_request(families, None, None, "", &mut litellm_params);
    let error = result.expect_err("the upload without a file is refused");
    (error, litellm_params)
}

fn upload_request(
    families: &Families,
    file: Option<&FileData>,
    purpose: Option<CreatePurpose>,
    model: &str,
    litellm_params: &mut RequestParams,
) -> Result<FileUploadHandoff, BatchRecordsError> {
    let sources = Sources::load();
    let aliases = BTreeMap::new();
    let lookup = StubLookup;
    let secrets = secret_lookup();
    let optional_params = RequestParams::new();
    let resolution = sources.resolution();
    let context = BatchRecordContext {
        aliases: &aliases,
        families,
        lookup: &lookup,
        resolution: &resolution,
    };
    let mut request = CreateFileRequest {
        model,
        file,
        purpose,
        optional_params: &optional_params,
        litellm_params,
        environment: &sources.environment,
        secrets: &secrets,
        region_inputs: RegionInputs {
            environment_region_name: None,
            environment_region: None,
            session_region: None,
        },
        id: FIXED_UUID,
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    };
    runtime().block_on(transform_create_file_request(
        &mut request,
        &context,
        resolve_credentials,
    ))
}

#[test]
fn replays_the_bedrock_services_batch_records_corpus() {
    let directory = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/bedrock_services_batch_records"
    );
    let mut seeds = 0_usize;
    for entry in std::fs::read_dir(directory).expect("the corpus directory reads") {
        let path = entry.expect("the corpus entry reads").path();
        let bytes = std::fs::read(&path).expect("the seed reads");
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("the seed is named");
        let outcome = decode_batch_input_line(&bytes);
        match name {
            "valid-chat" => {
                let record = outcome
                    .expect("the valid seed decodes")
                    .expect("the valid seed is a record");
                assert_eq!(record.custom_id, CustomId::Text(String::from("a")));
                assert_eq!(record.kind, BatchRecordKind::Chat);
            }
            "null-custom-id" => {
                let record = outcome
                    .expect("the null seed decodes")
                    .expect("the null seed is a record");
                assert_eq!(record.custom_id, CustomId::Null);
            }
            "boundary-whitespace" => {
                let record = outcome
                    .expect("the boundary seed decodes")
                    .expect("the boundary seed is a record");
                assert_eq!(record.custom_id, CustomId::Text(String::from("b")));
            }
            "blank-line" => assert!(matches!(outcome, Ok(None))),
            "malformed-json" | "truncated" => {
                outcome.expect_err("the malformed seed is refused");
            }
            other => panic!("unexpected seed {other}"),
        }
        seeds = seeds.saturating_add(1);
    }
    assert!(seeds >= 6, "the corpus holds {seeds} seeds");
}
