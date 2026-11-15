#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_db::auxiliary_repository::file_content::ManagedFileContentRepository;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::json;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_chat_options::RequestAssembly;
use crate::api::responses_chat_options::capabilities::CompletionBridgeProbe;
use crate::api::responses_chat_options::records::ChatCompletionRequest;
use crate::responses_bridge::{BridgeRequest, ModelInfoError};

use super::batch::{BatchBodyError, responses_batch_to_chat_body};
use super::storage::{
    FileContentRepository, FileNamingStrategy, FileStorageBackend, LiteLlmDbStorageBackend,
    StorageError, UploadOptions, storage_url_to_row_id,
};

const MISSING_INPUT: &str =
    "Batch record for /v1/responses is missing required `input` field: model=";

#[derive(Debug, Clone, PartialEq, Eq)]
enum RepoCall {
    Store(Vec<u8>),
    Load(String),
    Delete(String),
}

struct RepoState {
    contents: BTreeMap<String, Vec<u8>>,
    calls: Vec<RepoCall>,
    row_id: String,
    failing: bool,
}

struct FakeRepository(Mutex<RepoState>);

impl FakeRepository {
    fn new(row_id: &str) -> Self {
        Self(Mutex::new(RepoState {
            contents: BTreeMap::new(),
            calls: Vec::new(),
            row_id: row_id.to_owned(),
            failing: false,
        }))
    }

    fn state(&self) -> MutexGuard<'_, RepoState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn calls(&self) -> Vec<RepoCall> {
        self.state().calls.clone()
    }

    fn put(&self, row_id: &str, content: &[u8]) {
        self.state()
            .contents
            .insert(row_id.to_owned(), content.to_vec());
    }

    fn fail(&self) {
        self.state().failing = true;
    }
}

#[async_trait]
impl FileContentRepository for FakeRepository {
    async fn store_content(&self, content: &[u8]) -> Result<String, TableError> {
        let mut state = self.state();
        state.calls.push(RepoCall::Store(content.to_vec()));
        if state.failing {
            return Err(TableError::Publish {
                message: "publish failed".to_owned(),
            });
        }
        let row_id = state.row_id.clone();
        state.contents.insert(row_id.clone(), content.to_vec());
        Ok(row_id)
    }

    async fn load_content(&self, row_id: &str) -> Result<Option<Vec<u8>>, TableError> {
        let mut state = self.state();
        state.calls.push(RepoCall::Load(row_id.to_owned()));
        if state.failing {
            return Err(TableError::Publish {
                message: "publish failed".to_owned(),
            });
        }
        Ok(state.contents.get(row_id).cloned())
    }

    async fn delete_content(&self, row_id: &str) -> Result<(), TableError> {
        let mut state = self.state();
        state.calls.push(RepoCall::Delete(row_id.to_owned()));
        if state.failing {
            return Err(TableError::Publish {
                message: "publish failed".to_owned(),
            });
        }
        state.contents.remove(row_id);
        Ok(())
    }
}

struct NoParameters;

impl SupportedParameterSource for NoParameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

struct NoProbe;

impl CompletionBridgeProbe for NoProbe {
    fn bridges_back(&self, _request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError> {
        Ok(false)
    }
}

fn cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}

fn parse(body: &str) -> Json {
    serde_json::from_str(body).unwrap()
}

fn batch(parsed: &Json, provider: Option<&str>) -> Result<ChatCompletionRequest, BatchBodyError> {
    let cache = cache();
    let parameters = NoParameters;
    let probe = NoProbe;
    let mut assembly = RequestAssembly {
        limits: ApiLimits::default(),
        cache: &cache,
        parameters: &parameters,
        probe: &probe,
        logging: None,
        route_all_openai_to_responses: false,
    };
    responses_batch_to_chat_body(parsed, provider, &mut assembly)
}

struct MemoryBackend {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl FileStorageBackend for MemoryBackend {
    type Error = StorageError;

    async fn upload_file(
        &self,
        _file_content: &[u8],
        _filename: &str,
        _content_type: &str,
        _options: UploadOptions<'_>,
    ) -> Result<String, StorageError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("upload".to_owned());
        Ok("memory://stored".to_owned())
    }

    async fn download_file(&self, storage_url: &str) -> Result<Vec<u8>, StorageError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("download".to_owned());
        Ok(storage_url.as_bytes().to_vec())
    }
}

#[tokio::test]
async fn database_bytes_lifecycle() {
    let options = UploadOptions::default();
    assert_eq!(options.path_prefix, None);
    assert_eq!(options.file_naming_strategy, FileNamingStrategy::Uuid);
    let backend = LiteLlmDbStorageBackend {
        contents: FakeRepository::new("r"),
    };
    let url = backend
        .upload_file(
            b"\x00\xff",
            "a",
            "text/plain",
            UploadOptions {
                path_prefix: Some("p"),
                file_naming_strategy: FileNamingStrategy::Timestamp,
            },
        )
        .await
        .unwrap();
    assert_eq!(url, "litellm_db://r");
    let mut expected = vec![RepoCall::Store(vec![0_u8, 255])];
    assert_eq!(backend.contents.calls(), expected);

    let content = backend.download_file("litellm_db://r").await.unwrap();
    assert_eq!(content, vec![0_u8, 255]);
    expected.push(RepoCall::Load("r".to_owned()));
    assert_eq!(backend.contents.calls(), expected);

    backend.delete_file("litellm_db://r").await.unwrap();
    backend.delete_file("litellm_db://r").await.unwrap();
    expected.push(RepoCall::Delete("r".to_owned()));
    expected.push(RepoCall::Delete("r".to_owned()));
    assert_eq!(backend.contents.calls(), expected);
}

#[tokio::test]
async fn database_empty_and_absent_content() {
    let backend = LiteLlmDbStorageBackend {
        contents: FakeRepository::new("r"),
    };
    let missing = backend.download_file("litellm_db://r").await.unwrap_err();
    assert_eq!(
        missing.to_string(),
        "No stored file content for litellm_db://r"
    );
    assert!(matches!(missing, StorageError::NoStoredContent { .. }));
    backend.contents.put("empty", b"");
    let empty = backend.download_file("litellm_db://empty").await.unwrap();
    assert!(empty.is_empty());
    assert_eq!(
        backend.contents.calls(),
        vec![
            RepoCall::Load("r".to_owned()),
            RepoCall::Load("empty".to_owned())
        ]
    );
}

#[tokio::test]
async fn repository_failures_propagate() {
    let backend = LiteLlmDbStorageBackend {
        contents: FakeRepository::new("r"),
    };
    backend.contents.fail();
    let upload = backend
        .upload_file(b"x", "a", "text/plain", UploadOptions::default())
        .await
        .unwrap_err();
    assert_eq!(
        upload.to_string(),
        "config sync publication failed: publish failed"
    );
    assert!(matches!(upload, StorageError::Repository(_)));
    let delete = backend.delete_file("litellm_db://r").await.unwrap_err();
    assert!(matches!(delete, StorageError::Repository(_)));
    assert_eq!(
        backend.contents.calls(),
        vec![
            RepoCall::Store(b"x".to_vec()),
            RepoCall::Delete("r".to_owned())
        ]
    );
}

#[tokio::test]
async fn managed_repository_port_delegates() {
    let backend = LiteLlmDbStorageBackend {
        contents: ManagedFileContentRepository::new(
            None,
            Arc::new(|_object: &str| Box::pin(async { Ok(()) })),
        ),
    };
    let upload = backend
        .upload_file(b"x", "a", "text/plain", UploadOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(
        upload,
        StorageError::Repository(TableError::Database(_))
    ));
    let download = backend.download_file("litellm_db://r").await.unwrap_err();
    assert!(matches!(
        download,
        StorageError::Repository(TableError::Database(_))
    ));
    let delete = backend.delete_file("litellm_db://r").await.unwrap_err();
    assert!(matches!(
        delete,
        StorageError::Repository(TableError::Database(_))
    ));
}

#[test]
fn storage_uri_validation() {
    assert_eq!(storage_url_to_row_id("litellm_db://r").unwrap(), "r");
    assert_eq!(storage_url_to_row_id("litellm_db://").unwrap(), "");
    assert_eq!(
        storage_url_to_row_id("litellm_db://a%2Fb").unwrap(),
        "a%2Fb"
    );
    let https = storage_url_to_row_id("https://example.invalid/r").unwrap_err();
    assert_eq!(
        https.to_string(),
        "Not a litellm_db storage url: https://example.invalid/r"
    );
    assert!(matches!(https, StorageError::NotLiteLlmDbUrl { .. }));
    let upper = storage_url_to_row_id("LITELLM_DB://r").unwrap_err();
    assert_eq!(
        upper.to_string(),
        "Not a litellm_db storage url: LITELLM_DB://r"
    );
}

#[tokio::test]
async fn default_delete_noop() {
    let backend = MemoryBackend {
        calls: Mutex::new(Vec::new()),
    };
    backend.delete_file("memory://stored").await.unwrap();
    assert!(
        backend
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

#[test]
fn responses_batch_projection() {
    let body = r#"{"model":"m","input":"hello","instructions":"system","max_output_tokens":7,"temperature":0.2,"metadata":{"tag":"a"},"stream":true,"unknown":"drop"}"#;
    let parsed = parse(body);
    let request = batch(&parsed, Some("openai")).unwrap();
    let expected = serde_json::from_str(
        r#"{"model":"m","messages":[{"role":"system","content":"system"},{"role":"user","content":"hello"}],"max_tokens":7,"temperature":0.2,"metadata":{"tag":"a"},"custom_llm_provider":"openai"}"#,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    let repeated = batch(&parsed, Some("openai")).unwrap();
    assert_eq!(
        serde_json::to_value(&repeated).unwrap(),
        serde_json::to_value(&request).unwrap()
    );
    assert_eq!(parse(body), parsed);
}

#[test]
fn responses_batch_tools_and_images_forwarded() {
    let body = r#"{"model":"m","input":[{"role":"user","content":[{"type":"input_text","text":"hi"},{"type":"input_image","image_url":"https://example.invalid/i.png"}]}],"tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}"#;
    let request = batch(&parse(body), None).unwrap();
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(
        value.pointer("/tools/0/function/strict"),
        Some(&json!(false))
    );
    assert_eq!(
        value.pointer("/tools/0/function/description"),
        Some(&json!(""))
    );
    assert_eq!(
        value.pointer("/messages/0/content/1/image_url/detail"),
        Some(&json!("auto"))
    );
    assert_eq!(value.pointer("/custom_llm_provider"), None);
}

#[test]
fn responses_batch_missing_input() {
    let missing = batch(&parse(r#"{"model":"m"}"#), None).unwrap_err();
    assert_eq!(missing.to_string(), format!("{MISSING_INPUT}m"));
    assert!(matches!(missing, BatchBodyError::MissingInput { .. }));
    let null = batch(&parse(r#"{"model":"m","input":null}"#), None).unwrap_err();
    assert_eq!(null.to_string(), format!("{MISSING_INPUT}m"));
    let unnamed = batch(&parse("{}"), None).unwrap_err();
    assert_eq!(unnamed.to_string(), MISSING_INPUT);
}

#[test]
fn responses_batch_non_string_and_empty_input() {
    let numeric = batch(&parse(r#"{"model":123,"input":"hi"}"#), None).unwrap();
    let numeric = serde_json::to_value(&numeric).unwrap();
    assert_eq!(numeric.pointer("/model"), Some(&json!("")));

    let empty = batch(&parse(r#"{"model":"m","input":""}"#), None).unwrap();
    let expected =
        serde_json::from_str(r#"{"model":"m","messages":[{"role":"user","content":""}]}"#).unwrap();
    assert_eq!(serde_json::to_value(&empty).unwrap(), expected);
}

#[test]
fn responses_batch_rejects_non_object_body() {
    let array = batch(&parse("[1]"), None).unwrap_err();
    assert_eq!(
        array.to_string(),
        "a Responses batch body must be a JSON object"
    );
    assert!(matches!(array, BatchBodyError::BodyNotObject));
}

#[test]
fn responses_batch_names_the_wrong_field() {
    let wrong = batch(
        &parse(r#"{"model":"m","input":"hi","temperature":"hot"}"#),
        None,
    )
    .unwrap_err();
    assert_eq!(
        wrong.to_string(),
        "the temperature field cannot hold this value"
    );
    assert!(matches!(wrong, BatchBodyError::Fields { .. }));
}
