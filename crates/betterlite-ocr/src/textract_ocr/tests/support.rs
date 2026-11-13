use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use secrecy::SecretString;

use crate::json::{Json, Object};
use crate::ocr_auth::{AwsRequestSigner, AwsSigningRequest};
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::tests::support::{ocr_client, prepared_request};
use crate::ocr_http::{CallHooks, OcrClient, OcrWireRequest};
use crate::ocr_settings::OcrSettings;

pub(crate) fn document(source: &str) -> OcrDocument {
    OcrDocument::ImageUrl {
        image_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

pub(crate) fn request(model: &str, api_base: Option<&str>, params: Object) -> PreparedOcrRequest {
    let mut request = prepared_request(String::new(), params);
    request.model = model.to_owned();
    request.document = document("data:image/png;base64,aGVsbG8=");
    request.connection.api_base = api_base.map(str::to_owned);
    request
}

pub(crate) fn with_secret(request: &mut PreparedOcrRequest, name: &str, value: &str) {
    let _previous = request
        .connection
        .secrets
        .insert(name.to_owned(), SecretString::from(value));
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SignedCall {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) service: String,
    pub(crate) region: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct SigningLog {
    calls: Mutex<Vec<SignedCall>>,
}

impl SigningLog {
    pub(crate) fn calls(&self) -> Vec<SignedCall> {
        self.calls
            .lock()
            .expect("the signing log lock is not poisoned")
            .clone()
    }
}

pub(crate) struct RecordingSigner {
    pub(crate) log: Arc<SigningLog>,
}

#[async_trait]
impl AwsRequestSigner for RecordingSigner {
    async fn sign(
        &self,
        request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        self.log
            .calls
            .lock()
            .expect("the signing log lock is not poisoned")
            .push(SignedCall {
                method: request.method.to_owned(),
                url: request.url.to_owned(),
                service: request.service.to_owned(),
                region: request.region.to_owned(),
                headers: request.headers.to_vec(),
                body: request.body.to_vec(),
            });
        Ok(vec![(
            "Authorization".to_owned(),
            "AWS4-HMAC-SHA256 recorded".to_owned(),
        )])
    }
}

pub(crate) fn signing_client() -> (Arc<SigningLog>, OcrClient) {
    let log = Arc::new(SigningLog::default());
    let signer = Arc::new(RecordingSigner {
        log: Arc::clone(&log),
    });
    let mut client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    client.aws = signer;
    (log, client)
}

pub(crate) fn body_bytes(outbound: &OutboundRequest) -> Vec<u8> {
    let RequestBody::Json(bytes) = &outbound.body else {
        panic!("the Textract body is JSON");
    };
    bytes.clone()
}

pub(crate) fn body_json(outbound: &OutboundRequest) -> Json {
    serde_json::from_slice(&body_bytes(outbound)).expect("the Textract body is JSON")
}

pub(crate) fn header(outbound: &OutboundRequest, name: &str) -> Option<String> {
    outbound
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON parses")
}

pub(crate) fn region_params(region: &str) -> Object {
    let mut params = Object::new();
    let _previous = params.insert("aws_region_name".to_owned(), Json::Text(region.to_owned()));
    params
}

pub(crate) struct NoHooks;

#[async_trait]
impl CallHooks for NoHooks {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(request)
    }

    async fn response_received(&self, _body: &[u8]) -> Result<(), OcrError> {
        Ok(())
    }
}
