#![cfg(test)]

use std::collections::BTreeMap;
use std::iter::empty;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::vec::IntoIter;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::file_type_records::{ByteStream, FileContentStreamingResult};
use betterlite_openai::files::records::{
    BinaryContent, CreatePurpose, FileContentRequest, FileCreate, FileDeleted, FileKind,
    FileObject, FilePage, FileUpload, FilesPurpose,
};
use http::Method;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::base::BaseConfig;
use super::config::{
    BaseFilesConfig, CompleteUrlRequest, CreateFileBody, FileLogging, FileOpenAiParam, FileParams,
    FileResponse, FileUrlRequest, Headers, NextPageRequest, PreparedRequest, StreamContext,
};
use super::endpoints::{BaseFileEndpoints, FileListRequest};
use super::error::FileContractError;
use super::upload_stream::BaseFileUploadStream;

#[derive(Default)]
struct Config {
    complete_url: CompleteUrl,
}

#[derive(Default)]
enum CompleteUrl {
    #[default]
    Default,
    Recording(Arc<Mutex<Vec<RecordedCall>>>),
}

#[derive(Debug, PartialEq)]
struct RecordedCall {
    api_base: Option<String>,
    api_key: Option<String>,
    model: String,
    optional_params: FileParams,
    litellm_params: FileParams,
}

struct BaseDefaults;

impl BaseConfig for BaseDefaults {}

impl BaseConfig for Config {
    fn get_complete_url(
        &self,
        request: CompleteUrlRequest<'_>,
    ) -> Result<String, FileContractError> {
        match &self.complete_url {
            CompleteUrl::Default => BaseDefaults.get_complete_url(request),
            CompleteUrl::Recording(calls) => {
                let call = RecordedCall {
                    api_base: request.api_base.map(str::to_owned),
                    api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                    model: request.model.to_owned(),
                    optional_params: request.optional_params.clone(),
                    litellm_params: request.litellm_params.clone(),
                };
                let mut recorded = calls.lock().unwrap_or_else(PoisonError::into_inner);
                recorded.push(call);
                drop(recorded);
                Ok(String::from("https://mock.invalid/v1/files"))
            }
        }
    }
}

type Prepared = Result<PreparedRequest, FileContractError>;
type Reply<T> = Result<T, FileContractError>;

impl BaseFilesConfig for Config {
    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn get_supported_openai_params(&self, _: &str) -> Vec<FileOpenAiParam> {
        vec![FileOpenAiParam::Purpose]
    }

    fn transform_create_file_request(
        &self,
        _: &str,
        _: &FileCreate,
        _: &FileParams,
        _: &FileParams,
    ) -> Result<CreateFileBody, FileContractError> {
        Ok(CreateFileBody::Text(String::new()))
    }

    #[rustfmt::skip]
    fn transform_create_file_response(&self, _: Option<&str>, _: &FileResponse, _: &mut FileLogging, _: &FileParams) -> Reply<FileObject> {
        Ok(file_object())
    }

    fn transform_retrieve_file_request(&self, _: &str, _: &FileParams, _: &FileParams) -> Prepared {
        Ok((String::new(), FileParams::new()))
    }

    #[rustfmt::skip]
    fn transform_retrieve_file_response(&self, _: &FileResponse, _: &mut FileLogging, _: &FileParams) -> Reply<FileObject> {
        Ok(file_object())
    }

    fn transform_delete_file_request(&self, _: &str, _: &FileParams, _: &FileParams) -> Prepared {
        Ok((String::new(), FileParams::new()))
    }

    #[rustfmt::skip]
    fn transform_delete_file_response(&self, _: &FileResponse, _: &mut FileLogging, _: &FileParams) -> Reply<FileDeleted> {
        Ok(file_deleted())
    }

    fn transform_list_files_request(
        &self,
        _: Option<&str>,
        _: &FileParams,
        _: &FileParams,
    ) -> Prepared {
        Ok((String::new(), FileParams::new()))
    }

    #[rustfmt::skip]
    fn transform_list_files_response(&self, _: &FileResponse, _: &mut FileLogging, _: &FileParams) -> Reply<Vec<FileObject>> {
        Ok(Vec::new())
    }

    fn transform_file_content_request(
        &self,
        _: &FileContentRequest,
        _: &FileParams,
        _: &FileParams,
    ) -> Prepared {
        Ok((String::new(), FileParams::new()))
    }

    #[rustfmt::skip]
    fn transform_file_content_response(&self, _: &FileResponse, _: &mut FileLogging, _: &FileParams) -> Reply<BinaryContent> {
        Ok(binary_content())
    }
}

struct Counted {
    chunks: IntoIter<Vec<u8>>,
    consumed: Arc<AtomicUsize>,
}

impl Iterator for Counted {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        let chunk = self.chunks.next();
        if chunk.is_some() {
            self.consumed.fetch_add(1, Ordering::SeqCst);
        }
        chunk
    }
}

struct Upload {
    chunks: Vec<Vec<u8>>,
}

impl BaseFileUploadStream for Upload {
    fn iter_bytes(&self) -> ByteStream {
        Box::new(self.chunks.clone().into_iter())
    }
}

struct EndpointFixture;

#[async_trait::async_trait]
impl BaseFileEndpoints for EndpointFixture {
    type Error = FileContractError;

    async fn acreate_file(
        &self,
        _create_file_request: &FileCreate,
        _target_model_names: &[String],
        _user_api_key_dict: &UserAPIKeyAuth,
    ) -> Result<FileObject, Self::Error> {
        Ok(file_object())
    }

    async fn afile_retrieve(&self, _file_id: &str) -> Result<FileObject, Self::Error> {
        Ok(file_object())
    }

    async fn afile_list(&self, _request: FileListRequest<'_>) -> Result<FilePage, Self::Error> {
        Ok(file_page())
    }

    async fn afile_delete(
        &self,
        _file_id: &str,
        _data: &FileParams,
    ) -> Result<FileDeleted, Self::Error> {
        Ok(file_deleted())
    }

    async fn afile_content(
        &self,
        _file_id: &str,
        _data: &FileParams,
    ) -> Result<BinaryContent, Self::Error> {
        Ok(binary_content())
    }
}

fn params(text: &str) -> FileParams {
    serde_json::from_str(text).unwrap()
}

fn create_file() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some(String::from("upload.jsonl")),
            content_type: Some(String::from("application/jsonl")),
            data: Vec::new(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn response() -> FileResponse {
    FileResponse {
        status_code: 200,
        headers: Headers::new(),
        body: br#"{"has_more": true}"#.to_vec(),
        url: Some(String::from("https://mock.invalid/v1/files")),
    }
}

fn file_object() -> FileObject {
    FileObject {
        id: String::from("file_1"),
        bytes: 0,
        created_at: 0,
        filename: String::from("upload.jsonl"),
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: None,
        expires_at: None,
        status_details: None,
    }
}

fn file_page() -> FilePage {
    FilePage {
        object: None,
        data: Vec::new(),
        has_more: None,
        first_id: None,
        last_id: None,
    }
}

fn file_deleted() -> FileDeleted {
    FileDeleted {
        id: String::from("file_1"),
        object: FileKind::File,
        deleted: true,
    }
}

fn binary_content() -> BinaryContent {
    BinaryContent {
        status: 200,
        headers: IndexMap::new(),
        body: Vec::new(),
    }
}

fn auth() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap()
}

#[test]
fn base_defaults() {
    let config = Config::default();
    assert_eq!(config.file_upload_http_method(), Method::POST);
    let optional_params = params(r#"{"after": "f"}"#);
    let litellm_params = params(r#"{"cursor": "c"}"#);
    let next =
        config.transform_list_files_next_request(&response(), &optional_params, &litellm_params);
    assert_eq!(next, None);
}

#[test]
fn file_url_forwarding() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let config = Config {
        complete_url: CompleteUrl::Recording(Arc::clone(&calls)),
    };
    let optional_params = params(r#"{"purpose": "batch"}"#);
    let litellm_params = params(r#"{"custom_llm_provider": "openai"}"#);
    let api_key = SecretString::from("sk-test");
    let data = create_file();
    let url = config
        .get_complete_file_url(FileUrlRequest {
            api_base: Some("https://mock.invalid/v1"),
            api_key: Some(&api_key),
            model: "file-model",
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            data: &data,
        })
        .unwrap();
    assert_eq!(url, "https://mock.invalid/v1/files");
    let recorded = calls.lock().unwrap();
    assert_eq!(
        *recorded,
        vec![RecordedCall {
            api_base: Some(String::from("https://mock.invalid/v1")),
            api_key: Some(String::from("sk-test")),
            model: String::from("file-model"),
            optional_params: params(r#"{"purpose": "batch"}"#),
            litellm_params: params(r#"{"custom_llm_provider": "openai"}"#),
        }]
    );
    drop(recorded);
    assert_missing_api_base(&data);
}

fn assert_missing_api_base(data: &FileCreate) {
    let config = Config::default();
    let optional_params = params(r#"{"purpose": "batch"}"#);
    let litellm_params = params(r#"{"custom_llm_provider": "openai"}"#);
    let error = config
        .get_complete_file_url(FileUrlRequest {
            api_base: None,
            api_key: None,
            model: "file-model",
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            data,
        })
        .unwrap_err();
    assert_eq!(error, FileContractError::ApiBaseRequired);
    assert_eq!(error.to_string(), "api_base is required");
}

#[test]
fn stream_passthrough_and_replay() {
    let config = Config::default();
    let headers = Headers::from([
        (String::from("content-type"), String::from("text/plain")),
        (String::from("x-provider"), String::from("test")),
    ]);
    let mut logging = FileLogging::default();
    let chunks = vec![b"a".to_vec(), b"\x00\xff".to_vec()];
    let consumed = Arc::new(AtomicUsize::new(0));
    let stream = Counted {
        chunks: chunks.clone().into_iter(),
        consumed: Arc::clone(&consumed),
    };
    let result = config.transform_file_content_stream(
        Box::new(stream),
        headers.clone(),
        StreamContext {
            request_url: "u",
            logging_obj: &mut logging,
            litellm_params: &params(r#"{"k": "v"}"#),
        },
    );
    assert_eq!(consumed.load(Ordering::SeqCst), 0);
    assert_eq!(result.headers, headers);
    assert_eq!(result.stream_iterator.collect::<Vec<_>>(), chunks);
    assert_eq!(consumed.load(Ordering::SeqCst), 2);
    let upload = Upload {
        chunks: vec![b"abc".to_vec()],
    };
    let mut first = upload.iter_bytes();
    let second = upload.iter_bytes();
    assert_eq!(first.next(), Some(b"abc".to_vec()));
    assert_eq!(first.next(), None);
    assert_eq!(second.collect::<Vec<_>>(), vec![b"abc".to_vec()]);
}

#[tokio::test]
async fn contract_surface() {
    let config = Config::default();
    config_surface(&config);
    endpoint_surface(&EndpointFixture).await;
}

fn config_surface(config: &Config) {
    let mut logging = FileLogging::default();
    let optional_params = FileParams::new();
    let _params: Vec<FileOpenAiParam> = config.get_supported_openai_params("file-model");
    let _create_body: CreateFileBody = config
        .transform_create_file_request(
            "file-model",
            &create_file(),
            &optional_params,
            &optional_params,
        )
        .unwrap();
    let _created: FileObject = config
        .transform_create_file_response(None, &response(), &mut logging, &optional_params)
        .unwrap();
    let _retrieve_request: PreparedRequest = config
        .transform_retrieve_file_request("file_1", &optional_params, &optional_params)
        .unwrap();
    let _retrieved: FileObject = config
        .transform_retrieve_file_response(&response(), &mut logging, &optional_params)
        .unwrap();
    let _delete_request: PreparedRequest = config
        .transform_delete_file_request("file_1", &optional_params, &optional_params)
        .unwrap();
    let _deleted: FileDeleted = config
        .transform_delete_file_response(&response(), &mut logging, &optional_params)
        .unwrap();
    let _list_request: PreparedRequest = config
        .transform_list_files_request(None, &optional_params, &optional_params)
        .unwrap();
    let _next: Option<NextPageRequest> =
        config.transform_list_files_next_request(&response(), &optional_params, &optional_params);
    let _listed: Vec<FileObject> = config
        .transform_list_files_response(&response(), &mut logging, &optional_params)
        .unwrap();
    config_content_surface(config, &mut logging, &optional_params);
}

fn config_content_surface(
    config: &Config,
    logging: &mut FileLogging,
    optional_params: &FileParams,
) {
    let _content_request: PreparedRequest = config
        .transform_file_content_request(&content_request(), optional_params, optional_params)
        .unwrap();
    let _content: BinaryContent = config
        .transform_file_content_response(&response(), logging, optional_params)
        .unwrap();
    let _stream: FileContentStreamingResult = config.transform_file_content_stream(
        Box::new(empty()),
        Headers::new(),
        StreamContext {
            request_url: "u",
            logging_obj: logging,
            litellm_params: optional_params,
        },
    );
}

fn content_request() -> FileContentRequest {
    FileContentRequest {
        file_id: String::from("file_1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

async fn endpoint_surface(endpoint: &EndpointFixture) {
    let create_request = create_file();
    let data = FileParams::new();
    let user = auth();
    let _created: FileObject = endpoint
        .acreate_file(&create_request, &[], &user)
        .await
        .unwrap();
    let _retrieved: FileObject = endpoint.afile_retrieve("file_1").await.unwrap();
    let _listed: FilePage = endpoint
        .afile_list(FileListRequest {
            purpose: None,
            user_api_key_dict: &user,
            limit: None,
            after: None,
            data: &data,
        })
        .await
        .unwrap();
    let _deleted: FileDeleted = endpoint.afile_delete("file_1", &data).await.unwrap();
    let _content: BinaryContent = endpoint.afile_content("file_1", &data).await.unwrap();
}
