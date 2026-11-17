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
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

use super::chat::{self, ChatCallInput};
use super::embedding::{self, EmbeddingCallInput};
use super::error::FamilyError;
use super::rerank::{self, RerankCallInput};
use super::{EmbeddingRerank, EmbeddingRerankPrepared, EmbeddingRerankResult};
use crate::api::rerank_contracts::ReturnDocuments;
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

fn key(value: &str) -> Option<SecretString> {
    Some(SecretString::from(value.to_owned()))
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn text_field(object: &Object, name: &str) -> Option<String> {
    object.get(name).and_then(Json::as_str).map(str::to_owned)
}

fn number_field(object: &Object, name: &str) -> Option<i64> {
    let Json::Number(number) = object.get(name)? else {
        return None;
    };
    number.as_i64()
}

fn flag_field(object: &Object, name: &str) -> Option<bool> {
    let Json::Bool(value) = object.get(name)? else {
        return None;
    };
    Some(*value)
}

fn items_field<'object>(object: &'object Object, name: &str) -> &'object [Json] {
    let Some(Json::List(items)) = object.get(name) else {
        return &[];
    };
    items
}

fn object_field<'object>(object: &'object Object, name: &str) -> Option<&'object Object> {
    object.get(name).and_then(Json::as_object)
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

struct ChatFixture {
    provider: ProviderId,
    model: String,
    request: ChatRequest,
    stream: bool,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    headers: Option<Headers>,
    extra: Headers,
    settings: Settings,
}

impl ChatFixture {
    fn new(model: &str) -> Self {
        Self {
            provider: ProviderId::Cohere,
            model: model.to_owned(),
            request: ChatRequest::parse(br#"{"messages":[{"role":"user","content":"hi"}]}"#)
                .unwrap(),
            stream: false,
            api_key: key("key"),
            api_base: None,
            headers: None,
            extra: Headers::new(),
            settings: Settings::default(),
        }
    }

    fn prepare(&self) -> Result<EmbeddingRerankPrepared, FamilyError> {
        chat::prepare(&ChatCallInput {
            provider: self.provider,
            model: &self.model,
            request: &self.request,
            stream: self.stream,
            api_key: self.api_key.as_ref(),
            api_base: self.api_base.as_deref(),
            headers: self.headers.as_ref(),
            extra_headers: &self.extra,
            settings: &self.settings,
            timeout: TIMEOUT,
        })
    }

    fn prepared(&self) -> EmbeddingRerankPrepared {
        self.prepare().unwrap()
    }
}

struct EmbeddingFixture {
    provider: ProviderId,
    model: String,
    request: EmbeddingRequest,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    headers: Option<Headers>,
    extra: Headers,
    settings: Settings,
    host: embedding_host_settings::Settings,
    environment: BTreeMap<String, String>,
}

impl EmbeddingFixture {
    fn new(provider: ProviderId, model: &str) -> Self {
        Self {
            provider,
            model: model.to_owned(),
            request: serde_json::from_str(r#"{"input":"hi"}"#).unwrap(),
            api_key: key("key"),
            api_base: None,
            headers: None,
            extra: Headers::new(),
            settings: Settings::default(),
            host: embedding_host_settings::Settings::default(),
            environment: BTreeMap::new(),
        }
    }

    fn prepare(&self) -> Result<EmbeddingRerankPrepared, FamilyError> {
        embedding::prepare(&EmbeddingCallInput {
            provider: self.provider,
            model: &self.model,
            request: &self.request,
            api_key: self.api_key.as_ref(),
            api_base: self.api_base.as_deref(),
            headers: self.headers.as_ref(),
            extra_headers: &self.extra,
            settings: &self.settings,
            host: &self.host,
            environment: &self.environment,
            timeout: TIMEOUT,
        })
    }

    fn prepared(&self) -> EmbeddingRerankPrepared {
        self.prepare().unwrap()
    }
}

struct RerankFixture {
    provider: ProviderId,
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    top_n: Option<i64>,
    rank_fields: Option<Vec<String>>,
    return_documents: ReturnDocuments,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    instruction: Option<String>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    headers: Option<Headers>,
    cohere: cohere_rerank_settings::Settings,
    host: jina_infinity_rerank::Settings,
    settings: Settings,
    environment: BTreeMap<String, String>,
}

impl RerankFixture {
    fn new(provider: ProviderId, model: &str) -> Self {
        Self {
            provider,
            model: model.to_owned(),
            query: String::from("what"),
            documents: vec![RerankDocument::Text(String::from("doc"))],
            top_n: None,
            rank_fields: None,
            return_documents: ReturnDocuments::Default,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
            api_key: key("key"),
            api_base: None,
            headers: None,
            cohere: cohere_rerank_settings::Settings::default(),
            host: jina_infinity_rerank::Settings::default(),
            settings: Settings::default(),
            environment: BTreeMap::new(),
        }
    }

    fn prepare(&self) -> Result<EmbeddingRerankPrepared, FamilyError> {
        rerank::prepare(&RerankCallInput {
            provider: self.provider,
            model: &self.model,
            query: &self.query,
            documents: &self.documents,
            top_n: self.top_n,
            rank_fields: self.rank_fields.as_deref(),
            return_documents: self.return_documents,
            max_chunks_per_doc: self.max_chunks_per_doc,
            max_tokens_per_doc: self.max_tokens_per_doc,
            instruction: self.instruction.as_deref(),
            api_key: self.api_key.as_ref(),
            api_base: self.api_base.as_deref(),
            headers: self.headers.as_ref(),
            cohere: &self.cohere,
            host: &self.host,
            settings: &self.settings,
            environment: &self.environment,
            timeout: TIMEOUT,
        })
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
