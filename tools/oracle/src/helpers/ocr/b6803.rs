use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_ocr::cohere_ocr::CohereParseConfig;
use betterlite_ocr::json::Object;
use betterlite_ocr::ocr_auth::{
    AwsRequestSigner, AwsSigningRequest, AzureTokenSource, VertexAccessToken, VertexTokenSource,
};
use betterlite_ocr::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use betterlite_ocr::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, PreparedOcrRequest, Secrets,
};
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_error::{OcrError, OcrSecretError};
use betterlite_ocr::ocr_http::{CallHooks, OcrClient, OcrWireRequest, SecretSource, ocr};
use betterlite_ocr::ocr_settings::OcrSettings;
use betterlite_ocr::response::OCRResponse;
use secrecy::SecretString;
use serde::Deserialize;

use crate::case::UpstreamAnswer;
use crate::helpers::{Helper, HelperError, HelperResult};
use crate::mock::{MockProvider, ProviderRequest};
use crate::routing_mock_scripts::{MockScript, Outcome, ProviderCall, ScriptedReply, Wait};

#[derive(Deserialize)]
pub(crate) struct Input {
    model: String,
    document: serde_json::Value,
    #[serde(default)]
    options: serde_json::Map<String, serde_json::Value>,
    response: serde_json::Value,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("Runtime", error.to_string()))?;
        runtime.block_on(call(input))
    }
}

async fn call(input: &Input) -> HelperResult {
    let mock = MockProvider::start(script(&input.response))
        .map_err(|error| failure("Mock", error.to_string()))?;
    let request = prepared_request(input, mock.base_url.clone())?;
    let client = client()?;
    let authorization = authorization_header(&request, &client).await?;
    let answer = ocr(&CohereParseConfig, &client, &request, &NoHooks)
        .await
        .map_err(|error| failure("Provider", error.to_string()))?;
    let calls = mock
        .finish()
        .map_err(|error| failure("Mock", error.to_string()))?;
    let Some(call) = calls.into_iter().next() else {
        return Err(failure(
            "Mock",
            "the Cohere provider sent no request".to_owned(),
        ));
    };
    output(call, &answer, authorization.as_deref())
}

async fn authorization_header(
    request: &PreparedOcrRequest,
    client: &OcrClient,
) -> Result<Option<String>, Box<HelperError>> {
    let headers = CohereParseConfig
        .validate_environment(request, client)
        .await
        .map_err(|error| failure("Headers", error.to_string()))?;
    Ok(headers
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value))
}

fn prepared_request(
    input: &Input,
    api_base: String,
) -> Result<PreparedOcrRequest, Box<HelperError>> {
    let document = serde_json::from_value::<OcrDocument>(input.document.clone())
        .map_err(|error| failure("Input", error.to_string()))?;
    let optional_params =
        serde_json::from_value::<Object>(serde_json::Value::Object(input.options.clone()))
            .map_err(|error| failure("Input", error.to_string()))?;
    Ok(PreparedOcrRequest {
        model: input.model.clone(),
        document,
        connection: connection(input, api_base),
        caller_document: false,
        optional_params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    })
}

fn connection(input: &Input, api_base: String) -> OcrConnection {
    let credentials = ResolvedOcrCredentials {
        api_key: input.api_key.clone().map(|value| OcrSourced {
            value: SecretString::from(value),
            source: OcrInputSource::Request,
        }),
        api_base: Some(OcrSourced {
            value: api_base,
            source: OcrInputSource::Request,
        }),
    };
    let transport = OcrTransportConfig {
        extra_headers: input
            .extra_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        extra_headers_source: OcrInputSource::Request,
        timeout: Some(Duration::from_secs(10)),
        max_response_bytes: OCR_RESPONSE_MAX_BYTES,
    };
    OcrConnection::new(
        credentials,
        transport,
        OcrSettings::default(),
        Secrets::new(),
    )
}

fn output(call: ProviderCall, answer: &OCRResponse, authorization: Option<&str>) -> HelperResult {
    let ProviderRequest {
        method,
        path,
        headers,
        body,
    } = call.request;
    let normalized =
        serde_json::to_value(answer).map_err(|error| failure("Response", error.to_string()))?;
    let native = serde_json::to_value(&answer.provider_native_response)
        .map_err(|error| failure("Response", error.to_string()))?;
    Ok(serde_json::json!({
        "request": {
            "method": method,
            "path": path,
            "authorization": authorization,
            "content_type": headers.get("content-type"),
            "body": body,
        },
        "response": normalized,
        "native": native,
    }))
}

fn script(response: &serde_json::Value) -> MockScript {
    MockScript {
        deadline: Duration::from_secs(10),
        replies: vec![ScriptedReply {
            wait: Wait::None,
            then: Outcome::Answer(UpstreamAnswer {
                status: 200,
                headers: BTreeMap::new(),
                body: Some(response.clone()),
                events: None,
            }),
        }],
        barriers: BTreeMap::new(),
    }
}

fn client() -> Result<OcrClient, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| failure("Proxy", error.to_string()))?,
    );
    let allowed = vec!["127.0.0.1".to_owned()];
    let validator = UrlValidator::new(SystemResolver, &allowed, &Verification::Enabled);
    Ok(OcrClient {
        provider_http: http_client(&policy)?,
        polling_http: http_client(&policy)?,
        document_http: http_client(&policy)?,
        document_fetcher: UserUrlFetcher {
            validator,
            validation: true,
        },
        settings: OcrSettings::default(),
        secrets: Arc::new(UnusedAuth),
        azure: Arc::new(UnusedAuth),
        aws: Arc::new(UnusedAuth),
        vertex: Arc::new(UnusedAuth),
    })
}

fn http_client(policy: &Arc<ProxyPolicy>) -> Result<ProxiedClient, Box<HelperError>> {
    let limits = TransportLimits {
        read_timeout: Duration::from_secs(10),
        ..TransportLimits::DEFAULT
    };
    ProxiedClient::build(base_builder(), policy, limits)
        .map_err(|error| failure("Http", error.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

struct NoHooks;

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

struct UnusedAuth;

impl SecretSource for UnusedAuth {
    fn secret(&self, _name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(None)
    }
}

#[async_trait]
impl AzureTokenSource for UnusedAuth {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

#[async_trait]
impl AwsRequestSigner for UnusedAuth {
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl VertexTokenSource for UnusedAuth {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex tokens in the b6803 helper"))
    }
}
