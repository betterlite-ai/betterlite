#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::BatchObject;

use super::BatchBody;
use super::config::{BaseBatchesConfig, BatchEnvironment, BatchUrlRequest};
use super::error::BatchContractError;
use crate::api::file_contracts::config::{FileLogging, FileParams, FileResponse, Headers};
use crate::api::file_contracts::error::BaseLlmException;

struct Config;

fn batch_text(id: &str) -> String {
    format!(
        "{{\"id\":\"{id}\",\"completion_window\":\"24h\",\"created_at\":1,\"endpoint\":\"/v1/chat/completions\",\"input_file_id\":\"file_1\",\"object\":\"batch\",\"status\":\"validating\"}}"
    )
}

fn batch_object(id: &str) -> BatchObject {
    serde_json::from_str(&batch_text(id)).unwrap()
}

fn create_request() -> CreateBatchRequest {
    CreateBatchRequest {
        completion_window: None,
        endpoint: None,
        input_file_id: Some("file_1".to_owned()),
        metadata: None,
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

fn params(text: &str) -> FileParams {
    serde_json::from_str(text).unwrap()
}

impl BaseBatchesConfig for Config {
    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn get_config(&self) -> FileParams {
        params("{\"max_retries\":3}")
    }

    fn validate_environment(
        &self,
        request: BatchEnvironment<'_>,
    ) -> Result<Headers, BatchContractError> {
        let mut headers = request.headers.clone();
        headers.insert("x-model".to_owned(), request.model.to_owned());
        Ok(headers)
    }

    fn get_complete_batch_url(
        &self,
        request: BatchUrlRequest<'_>,
    ) -> Result<String, BatchContractError> {
        let base = request.api_base.unwrap_or("https://mock.invalid");
        Ok(format!("{base}/v1/batches"))
    }

    fn transform_create_batch_request(
        &self,
        model: &str,
        create_batch_data: &CreateBatchRequest,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Result<BatchBody, BatchContractError> {
        let input_file_id = create_batch_data.input_file_id.clone().unwrap_or_default();
        Ok(BatchBody::Text(format!("{model}:{input_file_id}")))
    }

    fn transform_create_batch_response(
        &self,
        _model: Option<&str>,
        _raw: &FileResponse,
        _logging: &mut FileLogging,
        _params: &FileParams,
    ) -> Result<BatchObject, BatchContractError> {
        Ok(batch_object("batch_1"))
    }

    fn transform_retrieve_batch_request(
        &self,
        batch_id: &str,
        optional_params: &FileParams,
        _unused: &FileParams,
    ) -> Result<BatchBody, BatchContractError> {
        let mut bytes = batch_id.as_bytes().to_vec();
        bytes.extend(optional_params.keys().flat_map(|key| key.bytes()));
        Ok(BatchBody::Bytes(bytes))
    }

    fn transform_retrieve_batch_response(
        &self,
        _name: Option<&str>,
        raw_response: &FileResponse,
        _log: &mut FileLogging,
        _extra: &FileParams,
    ) -> Result<BatchObject, BatchContractError> {
        match raw_response.status_code {
            200 => serde_json::from_slice(&raw_response.body).map_err(|error| {
                let failure = self.get_error_class(&error.to_string(), 502, &raw_response.headers);
                BatchContractError::Upstream(failure)
            }),
            404 | 410 => Err(BatchContractError::Upstream(BaseLlmException {
                status_code: raw_response.status_code,
                message: "retrieve failed".to_owned(),
                headers: raw_response.headers.clone(),
            })),
            code => {
                let failure = self.get_error_class("retrieve failed", code, &raw_response.headers);
                Err(BatchContractError::Upstream(failure))
            }
        }
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }
    }
}

fn response(status_code: u16) -> FileResponse {
    FileResponse {
        status_code,
        headers: Headers::from([("retry-after".to_owned(), "3".to_owned())]),
        body: batch_text("batch_1").into_bytes(),
        url: None,
    }
}

#[test]
fn contract_requests() {
    let config: &dyn BaseBatchesConfig = &Config;
    assert_eq!(config.custom_llm_provider(), ProviderId::Openai);
    assert_eq!(config.get_config(), params("{\"max_retries\":3}"));
    let headers = Headers::from([("authorization".to_owned(), "Bearer key".to_owned())]);
    let empty = FileParams::new();
    let environment = config
        .validate_environment(BatchEnvironment {
            headers: &headers,
            model: "gpt-4o",
            messages: &[],
            optional_params: &empty,
            litellm_params: &empty,
            api_key: None,
            api_base: None,
        })
        .unwrap();
    assert_eq!(environment.len(), 2);
    assert_eq!(environment["x-model"], "gpt-4o");
    let data = create_request();
    let url = config
        .get_complete_batch_url(BatchUrlRequest {
            api_base: Some("https://api.example"),
            api_key: None,
            model: "gpt-4o",
            optional_params: &empty,
            litellm_params: &empty,
            data: &data,
        })
        .unwrap();
    assert_eq!(url, "https://api.example/v1/batches");
}

#[test]
fn contract_bodies() {
    let config: &dyn BaseBatchesConfig = &Config;
    let empty = FileParams::new();
    let data = create_request();
    assert_eq!(
        config
            .transform_create_batch_request("gpt-4o", &data, &empty, &empty)
            .unwrap(),
        BatchBody::Text("gpt-4o:file_1".to_owned())
    );
    assert_eq!(
        config
            .transform_retrieve_batch_request("batch_1", &params("{\"after\":1}"), &empty)
            .unwrap(),
        BatchBody::Bytes(b"batch_1after".to_vec())
    );
}

#[test]
fn contract_responses() {
    let config: &dyn BaseBatchesConfig = &Config;
    let mut logging = FileLogging::default();
    let empty = FileParams::new();
    let created = config
        .transform_create_batch_response(Some("gpt-4o"), &response(200), &mut logging, &empty)
        .unwrap();
    assert_eq!(created, batch_object("batch_1"));
    let garbled = FileResponse {
        body: b"{".to_vec(),
        ..response(200)
    };
    let broken = config
        .transform_retrieve_batch_response(None, &garbled, &mut logging, &empty)
        .unwrap_err();
    assert_eq!(
        broken.to_string(),
        "EOF while parsing an object at line 1 column 1"
    );
    let retrieved = config
        .transform_retrieve_batch_response(None, &response(200), &mut logging, &empty)
        .unwrap();
    assert_eq!(retrieved, batch_object("batch_1"));
    let failure = config
        .transform_retrieve_batch_response(None, &response(404), &mut logging, &empty)
        .unwrap_err();
    assert_eq!(
        failure,
        BatchContractError::Upstream(BaseLlmException {
            status_code: 404,
            message: "retrieve failed".to_owned(),
            headers: Headers::from([("retry-after".to_owned(), "3".to_owned())]),
        })
    );
    assert_eq!(failure.to_string(), "retrieve failed");
}
