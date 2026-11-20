#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::embedding_provider_settings::Settings;
use betterlite_config::sections::{
    cohere_rerank_settings, embedding_host_settings, jina_infinity_rerank,
};
use betterlite_config::types::ProxyTrust;
use betterlite_core::embedding_rerank_providers::rerank_config;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_prompts::arguments::json::{Json as Field, Object};
use http::HeaderMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

use super::chat;
use super::embedding;
use super::error::FamilyError;
use super::rerank::{self, FamilyRerankConfig};
use super::{Common, EmbeddingRerank, EmbeddingRerankPrepared, EmbeddingRerankResult};
use crate::api::rerank_contracts::{
    BaseRerankConfig as _, EnvironmentRequest, OptionalRerankParams, Params, ReturnDocuments,
    get_optional_rerank_params,
};
use crate::operation::{OperationError, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter as _;

mod adapter_cases;
mod chat_cases;
mod embedding_cases;
mod rerank_cases;
const EXCHANGE: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(30);

const V1_CHAT_REPLY: &str =
    r#"{"text":"hello","citations":[],"usage":{"tokens":{"input_tokens":1,"output_tokens":2}}}"#;
const V2_CHAT_REPLY: &str = r#"{"message":{"content":[{"type":"text","text":"hello"}]},"usage":{"tokens":{"input_tokens":1,"output_tokens":2}}}"#;
const COHERE_EMBEDDING_REPLY: &str =
    r#"{"embeddings":{"float":[[0.1,0.2]]},"meta":{"billed_units":{"input_tokens":2}}}"#;
const VOYAGE_EMBEDDING_REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.5],"index":0}],"model":"voyage-3","usage":{"total_tokens":3}}"#;
const JINA_EMBEDDING_REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.5],"index":0}],"model":"jina-embeddings-v3","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
const INFINITY_EMBEDDING_REPLY: &str = r#"{"object":"list","model":"infinity-model","data":[{"object":"embedding","embedding":[0.5],"index":0}],"usage":{"prompt_tokens":2,"total_tokens":2}}"#;
const RERANK_REPLY: &str =
    r#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5,"document":{"text":"doc"}}]}"#;
const VOYAGE_RERANK_REPLY: &str = r#"{"id":"r-1","data":[{"index":0,"relevance_score":0.5,"document":"doc"}],"usage":{"total_tokens":3}}"#;
const CHAT_REQUEST: &[u8] = br#"{"messages":[{"role":"user","content":"hi"}]}"#;

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn deployment(model: &str, api_base: &str, api_key: Option<&str>) -> Deployment {
    Deployment {
        model: model.to_owned(),
        api_base: api_base.to_owned(),
        endpoint: Url::parse(api_base).unwrap(),
        api_key: api_key.map(|value| SecretString::from(value.to_owned())),
        authorization: None,
        organization: None,
        timeout: TIMEOUT,
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
        anthropic: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn body_of(prepared: &EmbeddingRerankPrepared) -> Object {
    serde_json::from_slice(&prepared.0.body).unwrap()
}

fn header_of(prepared: &EmbeddingRerankPrepared, name: &str) -> Option<String> {
    prepared
        .0
        .headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

fn url_of(prepared: &EmbeddingRerankPrepared) -> String {
    prepared.0.url.to_string()
}

fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn text_field(object: &Object, name: &str) -> Option<String> {
    object.get(name).and_then(Field::as_str).map(str::to_owned)
}

fn number_field(object: &Object, name: &str) -> Option<i64> {
    let Field::Number(number) = object.get(name)? else {
        return None;
    };
    number.as_i64()
}

fn flag_field(object: &Object, name: &str) -> Option<bool> {
    let Field::Bool(value) = object.get(name)? else {
        return None;
    };
    Some(*value)
}

fn items_field<'object>(object: &'object Object, name: &str) -> &'object [Field] {
    let Some(Field::List(items)) = object.get(name) else {
        return &[];
    };
    items
}

fn object_field<'object>(object: &'object Object, name: &str) -> Option<&'object Object> {
    object.get(name).and_then(Field::as_object)
}

async fn serve(listener: &TcpListener, status: u16, reply: &[u8]) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8(request.clone()).unwrap();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let length = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ")?.parse::<usize>().ok())
                .unwrap_or(0);
            if body.len() >= length {
                break;
            }
        }
    }
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reply.len()
    );
    let mut response = head.into_bytes();
    response.extend_from_slice(reply);
    stream.write_all(&response).await.unwrap();
    String::from_utf8(request).unwrap()
}

// the deployment-wide inputs every fixture shares: who to call, with what credentials and settings
struct Setup {
    provider: ProviderId,
    model: String,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    headers: Option<Headers>,
    extra: Headers,
    settings: Settings,
    host: embedding_host_settings::Settings,
    environment: BTreeMap<String, String>,
}

impl Setup {
    fn new(provider: ProviderId, model: &str) -> Self {
        Self {
            provider,
            model: model.to_owned(),
            api_key: Some(key("key")),
            api_base: None,
            headers: None,
            extra: Headers::new(),
            settings: Settings::default(),
            host: embedding_host_settings::Settings::default(),
            environment: BTreeMap::new(),
        }
    }

    fn common(&self) -> Common<'_> {
        Common {
            provider: self.provider,
            model: &self.model,
            api_key: self.api_key.as_ref(),
            api_base: self.api_base.as_deref(),
            headers: self.headers.as_ref(),
            extra_headers: &self.extra,
            settings: &self.settings,
            host: &self.host,
            environment: &self.environment,
            timeout: TIMEOUT,
        }
    }
}

struct ChatFixture {
    setup: Setup,
    request: ChatRequest,
    stream: bool,
}

impl ChatFixture {
    fn new(model: &str) -> Self {
        Self {
            setup: Setup::new(ProviderId::Cohere, model),
            request: ChatRequest::parse(CHAT_REQUEST).unwrap(),
            stream: false,
        }
    }

    fn prepare(&self) -> Result<EmbeddingRerankPrepared, FamilyError> {
        chat::prepare(&self.setup.common(), &self.request, self.stream)
    }

    fn prepared(&self) -> EmbeddingRerankPrepared {
        self.prepare().unwrap()
    }
}

struct EmbeddingFixture {
    setup: Setup,
    request: EmbeddingRequest,
}

impl EmbeddingFixture {
    fn new(provider: ProviderId, model: &str) -> Self {
        Self {
            setup: Setup::new(provider, model),
            request: serde_json::from_str(r#"{"input":"hi"}"#).unwrap(),
        }
    }

    fn prepare(&self) -> Result<EmbeddingRerankPrepared, FamilyError> {
        embedding::prepare(&self.setup.common(), &self.request)
    }

    fn prepared(&self) -> EmbeddingRerankPrepared {
        self.prepare().unwrap()
    }
}

// the rerank-only inputs a case sets before it prepares, all at their LiteLLM defaults
struct RerankOptions {
    query: String,
    documents: Vec<RerankDocument>,
    top_n: Option<i64>,
    rank_fields: Option<Vec<String>>,
    return_documents: Option<ReturnDocuments>,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    instruction: Option<String>,
    cohere: cohere_rerank_settings::Settings,
    host: jina_infinity_rerank::Settings,
}

impl Default for RerankOptions {
    fn default() -> Self {
        Self {
            query: String::from("what"),
            documents: vec![RerankDocument::Text(String::from("doc"))],
            top_n: None,
            rank_fields: None,
            return_documents: None,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
            cohere: cohere_rerank_settings::Settings::default(),
            host: jina_infinity_rerank::Settings::default(),
        }
    }
}

struct RerankFixture {
    setup: Setup,
    options: RerankOptions,
}

impl RerankFixture {
    fn new(provider: ProviderId, model: &str) -> Self {
        Self {
            setup: Setup::new(provider, model),
            options: RerankOptions::default(),
        }
    }

    fn prepare(&self) -> Result<EmbeddingRerankPrepared, FamilyError> {
        let family = self.family()?;
        let params = self.params(&family)?;
        let supplied = self
            .setup
            .settings
            .resolve_headers(self.setup.headers.as_ref(), &Headers::new());
        let headers = family.validate_environment(EnvironmentRequest {
            headers: &supplied,
            model: &self.setup.model,
            api_key: self.setup.api_key.as_ref(),
            optional_params: Some(&params),
            litellm_params: None,
        })?;
        let endpoint = family.url(self.setup.api_base.as_deref())?;
        let body = family.transform_rerank_request(&self.setup.model, &params, &headers, None)?;
        rerank::prepare(family.config, &endpoint, &headers, &body, TIMEOUT)
    }

    fn family(&self) -> Result<FamilyRerankConfig<'_>, FamilyError> {
        let config = rerank_config(
            self.setup.provider,
            self.setup.api_base.as_deref(),
            &self.present_version_params(),
        )
        .ok_or_else(|| FamilyError::Unsupported {
            provider: self.setup.provider.to_string(),
        })?;
        Ok(FamilyRerankConfig {
            config,
            cohere: &self.options.cohere,
            host: &self.options.host,
            environment: &self.setup.environment,
        })
    }

    fn present_version_params(&self) -> Vec<String> {
        let mut present = Vec::new();
        if self.options.max_chunks_per_doc.is_some() {
            present.push(String::from("max_chunks_per_doc"));
        }
        if self.options.max_tokens_per_doc.is_some() {
            present.push(String::from("max_tokens_per_doc"));
        }
        present
    }

    fn params(&self, family: &FamilyRerankConfig<'_>) -> Result<Params, FamilyError> {
        let mut non_default = Params::new();
        if let Some(rank_fields) = self.options.rank_fields.as_deref() {
            non_default.insert(
                String::from("rank_fields"),
                Json::List(rank_fields.iter().cloned().map(Json::Text).collect()),
            );
        }
        get_optional_rerank_params(
            family,
            OptionalRerankParams {
                model: &self.setup.model,
                drop_params: false,
                query: &self.options.query,
                documents: &self.options.documents,
                custom_llm_provider: Some(<&'static str>::from(self.setup.provider)),
                top_n: self.options.top_n,
                rank_fields: self.options.rank_fields.as_deref(),
                return_documents: self
                    .options
                    .return_documents
                    .unwrap_or(ReturnDocuments::Default),
                max_chunks_per_doc: self.options.max_chunks_per_doc,
                max_tokens_per_doc: self.options.max_tokens_per_doc,
                instruction: self.options.instruction.as_deref(),
                non_default_params: Some(&mut non_default),
            },
        )
    }

    fn prepared(&self) -> EmbeddingRerankPrepared {
        self.prepare().unwrap()
    }
}

fn decode(
    prepared: &EmbeddingRerankPrepared,
    status: u16,
    body: &str,
) -> Result<OperationResult, OperationError> {
    EmbeddingRerank.decode(
        &PreparedRequest::EmbeddingRerank(prepared.clone()),
        Upstream::Buffered(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }),
    )
}

fn embedding_result(result: OperationResult) -> EmbeddingRerankResult {
    let OperationResult::EmbeddingRerank(answer) = result else {
        panic!("an embedding or rerank answer");
    };
    answer
}
