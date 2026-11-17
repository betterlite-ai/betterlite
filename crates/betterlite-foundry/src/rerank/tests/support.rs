use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank::RerankParams;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::rerank::auth::{ValidateEnvironmentRequest, validate_environment};
use crate::rerank::error::FoundryRerankError;
use crate::rerank::reply::MODEL_GROUP_PARAM;
use crate::rerank::request::{FoundryRerankRequest, transform_request};

pub(super) const MODEL: &str = "cohere-rerank-v3-multilingual";
pub(super) const QUERY: &str = "which document answers the question";
pub(super) const MULTILINGUAL: &str = "offer-cohere-rerank-mul-paygo";
pub(super) const ENGLISH: &str = "offer-cohere-rerank-eng-paygo";
pub(super) const SENTINEL: &str = "rerank-sentinel-value";
pub(super) const BODY: &str = r#"{"id":"rerank-id","results":[{"index":0,"relevance_score":0.987654321,"document":{"text":"first document"}},{"index":1,"relevance_score":0.123456}],"meta":{"api_version":{"version":"1"},"billed_units":{"search_units":1}}}"#;
pub(super) const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0004_0000,
    max_items: 10_000,
};

// the credential sources never answer for a case here, so a fixture that records nothing proves the resolver stayed offline
pub(super) struct FixtureSources {
    calls: Mutex<Vec<&'static str>>,
}

impl FixtureSources {
    pub(super) const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn took(&self) -> bool {
        !self
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    fn refuse(&self, source: &'static str) -> SourceFailure {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(source);
        SourceFailure {
            class: String::from("ValueError"),
            status: None,
            message: format!("the {source} source is offline in this fixture"),
        }
    }
}

impl CredentialSources for FixtureSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(self.refuse("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(self.refuse("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(self.refuse("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("default credential");
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(self.refuse("OIDC"))
    }
}

pub(super) fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn expected_headers(bearer: &str) -> Headers {
    headers(&[
        ("Authorization", bearer),
        ("accept", "application/json"),
        ("content-type", "application/json"),
    ])
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("fixture JSON should be an object")
}

pub(super) fn auth_request<'data>(
    api_key: Option<&'data SecretString>,
    environment: &'data BTreeMap<String, String>,
    global_api_key: Option<&'data SecretString>,
    params: Option<&'data Object>,
    sources: &'data dyn CredentialSources,
) -> ValidateEnvironmentRequest<'data> {
    ValidateEnvironmentRequest {
        headers: Headers::new(),
        api_key,
        params,
        environment,
        global_api_key,
        refresh_enabled: false,
        sources,
    }
}

pub(super) fn resolved(request: &ValidateEnvironmentRequest<'_>) -> Headers {
    validate_environment(request).expect("fixture credentials should resolve")
}

pub(super) fn failure(request: &ValidateEnvironmentRequest<'_>) -> FoundryRerankError {
    validate_environment(request).expect_err("fixture credentials should fail")
}

pub(super) fn document(text: &str) -> RerankDocument {
    RerankDocument::Text(text.to_owned())
}

pub(super) fn request() -> FoundryRerankRequest {
    FoundryRerankRequest {
        model: MODEL.to_owned(),
        query: QUERY.to_owned(),
        documents: vec![document("first document")],
        top_n: None,
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
    }
}

pub(super) fn body(request: &FoundryRerankRequest) -> RerankParams {
    transform_request(request).expect("fixture request should transform")
}

pub(super) fn expected_body(text: &str) -> RerankParams {
    let parsed: Json = serde_json::from_str(text).expect("fixture JSON should parse");
    let Json::Object(fields) = parsed else {
        panic!("a rerank body is an object");
    };
    fields.into_iter().collect()
}

pub(super) fn hidden(group: Option<&str>) -> BTreeMap<String, Json> {
    group.map_or_else(BTreeMap::new, |name| {
        BTreeMap::from([(MODEL_GROUP_PARAM.to_owned(), Json::Text(name.to_owned()))])
    })
}
