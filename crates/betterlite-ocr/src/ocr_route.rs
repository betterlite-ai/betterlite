mod tests;

use crate::ocr_config::{AzureAdTokenProvider, Secrets};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::ocr_input::{LiteLLMOcrRequest, OcrDocumentInput};
use crate::ocr_prepare::{prepare_document, prepare_request};
use crate::ocr_provider::OcrConfigKind;
use crate::ocr_provider::handler::{OcrInterceptors, perform_ocr_request};
use crate::response::OCRResponse;
use async_trait::async_trait;
use secrecy::SecretString;
use std::sync::Arc;
use tokio::task::JoinSet;
use tracing::Instrument as _;

pub struct OcrRoute {
    pub client: OcrClient,
}

impl OcrRoute {
    pub async fn execute(
        &self,
        request: LiteLLMOcrRequest<OcrDocumentInput>,
        hooks: &dyn OcrInterceptors,
    ) -> Result<OCRResponse, OcrError> {
        let kind = request.config;
        let caller_document = matches!(&request.document, OcrDocumentInput::Document(_));
        let resolved = resolve_document(request)
            .instrument(tracing::debug_span!("litellm.prepare"))
            .await?;
        let secrets = resolved_secrets(&self.client, kind)?;
        let prepared = prepare_request(resolved, caller_document, &self.client, secrets);
        perform_ocr_request(&self.client, kind, prepared, hooks).await
    }
}

pub struct OcrCall {
    pub request: LiteLLMOcrRequest<OcrDocumentInput>,
    pub caller_token: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrOp {
    AcquireAzureAdToken,
}

#[async_trait]
pub trait OcrHost: Send + Sync {
    async fn call(&self, op: OcrOp) -> Result<SecretString, OcrError>;
}

pub struct OcrMachine {
    pub route: OcrRoute,
    pub call: OcrCall,
    pub host: Option<Arc<dyn OcrHost>>,
}

impl OcrMachine {
    pub async fn execute(self, hooks: &dyn OcrInterceptors) -> Result<OCRResponse, OcrError> {
        let Self { route, call, host } = self;
        let request = install_caller_token(call, host)?;
        route.execute(request, hooks).await
    }
}

fn install_caller_token(
    call: OcrCall,
    host: Option<Arc<dyn OcrHost>>,
) -> Result<LiteLLMOcrRequest<OcrDocumentInput>, OcrError> {
    let provided: Option<Arc<dyn AzureAdTokenProvider>> = match (call.caller_token, host) {
        (true, Some(token_host)) => Some(Arc::new(HostTokenProvider { host: token_host })),
        (true, None) => {
            return Err(OcrError::InvalidRequest(
                "a caller token call needs a host".to_owned(),
            ));
        }
        (false, _) => None,
    };
    let mut request = call.request;
    request.azure_ad_token_provider = provided.or(request.azure_ad_token_provider);
    Ok(request)
}

struct HostTokenProvider {
    host: Arc<dyn OcrHost>,
}

#[async_trait]
impl AzureAdTokenProvider for HostTokenProvider {
    async fn token(&self) -> Result<SecretString, OcrError> {
        self.host.call(OcrOp::AcquireAzureAdToken).await
    }
}

pub async fn aocr(
    client: OcrClient,
    request: LiteLLMOcrRequest<OcrDocumentInput>,
) -> Result<OCRResponse, OcrError> {
    let machine = OcrMachine {
        route: OcrRoute { client },
        call: OcrCall {
            request,
            caller_token: false,
        },
        host: None,
    };
    machine.execute(&()).await
}

async fn resolve_document(
    request: LiteLLMOcrRequest<OcrDocumentInput>,
) -> Result<LiteLLMOcrRequest<OcrDocument>, OcrError> {
    if let OcrDocumentInput::Document(_) = &request.document {
        return request.map_document(prepare_document);
    }
    let mut tasks = JoinSet::new();
    let _handle = tasks.spawn_blocking(move || request.map_document(prepare_document));
    let joined = tasks
        .join_next()
        .await
        .ok_or(OcrError::Unsupported("the OCR document task did not start"))?;
    joined.map_err(OcrError::DocumentTask)?
}

fn resolved_secrets(client: &OcrClient, kind: OcrConfigKind) -> Result<Secrets, OcrError> {
    let mut secrets = Secrets::new();
    for name in kind.secret_names() {
        if let Some(secret) = client.secrets.secret(name).map_err(OcrError::Secret)? {
            let _previous = secrets.insert(name.to_owned(), secret);
        }
    }
    Ok(secrets)
}
