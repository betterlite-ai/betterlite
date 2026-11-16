use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object};
use betterlite_openai::reasoning_models::{
    DropParams, ReasoningCapabilities, ReasoningError, catalogue_declares_default,
};
use betterlite_prompts::arguments::json::{Json, Object as JsonObject};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use crate::chat::capabilities::{
    CostMap, FoundryCapabilities, RequestParams, map_params, model_map_lookup_name,
    supported_params,
};
use crate::chat::environment::{EnvironmentRequest, TokenSource, validate_environment};
use crate::chat::environment_error::EnvironmentError;

pub(super) const SENTINEL_KEY: &str = "fixture-key-sentinel-8f2c";
pub(super) const SENTINEL_SECRET: &str = "fixture-client-secret-sentinel-41d7";
pub(super) const SENTINEL_TOKEN: &str = "fixture-ad-token-sentinel-9b3e";

#[must_use]
pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture object")
}

#[must_use]
pub(super) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("the fixture messages")
}

#[must_use]
pub(super) fn body(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture body")
}

#[must_use]
pub(super) fn catalog(chat: &[&str], text: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: chat.iter().map(|model| (*model).to_owned()).collect(),
        text_models: text.iter().map(|model| (*model).to_owned()).collect(),
    }
}

#[must_use]
pub(super) fn model_set(models: &[&str]) -> BTreeSet<String> {
    models.iter().map(|model| (*model).to_owned()).collect()
}

// A bare name answers only when the OpenAI catalog lists it, as LiteLLM's provider resolution does
pub(super) struct CapabilityFixture {
    entries: BTreeMap<String, JsonObject>,
    resolvable: BTreeSet<String>,
}

pub(super) struct CostEntries {
    keys: BTreeSet<String>,
}

impl CostMap for CostEntries {
    fn contains(&self, model: &str) -> bool {
        self.keys.contains(model)
    }
}

pub(super) struct Fixture {
    capabilities: CapabilityFixture,
    costs: CostEntries,
}

#[must_use]
pub(super) fn capabilities(source: &str, chat: &[&str], text: &[&str]) -> Fixture {
    let entries: BTreeMap<String, JsonObject> =
        serde_json::from_str(source).expect("the fixture model cost map");
    let keys = entries.keys().cloned().collect();
    let resolvable = chat
        .iter()
        .chain(text)
        .map(|model| (*model).to_owned())
        .collect();
    Fixture {
        capabilities: CapabilityFixture {
            entries,
            resolvable,
        },
        costs: CostEntries { keys },
    }
}

impl Fixture {
    pub(super) fn lookup_name(&self, model: &str) -> String {
        model_map_lookup_name(model, &self.costs)
    }

    pub(super) fn wrapper(&mut self) -> FoundryCapabilities<'_, CapabilityFixture> {
        FoundryCapabilities {
            inner: &mut self.capabilities,
            cost_map: &self.costs,
        }
    }

    pub(super) fn supported(
        &mut self,
        model: &str,
        catalog: &CatalogCapabilities,
    ) -> Vec<&'static str> {
        runtime().block_on(supported_params(
            &mut self.capabilities,
            model,
            catalog,
            &self.costs,
        ))
    }

    pub(super) fn mapped(
        &mut self,
        model: &str,
        requested: &str,
        drop_params: DropParams,
        catalog: &CatalogCapabilities,
    ) -> Result<Object, ReasoningError> {
        let mut non_default = object(requested);
        let mut optional = Object::new();
        runtime().block_on(map_params(
            &mut self.capabilities,
            &mut RequestParams {
                model,
                non_default_params: &mut non_default,
                optional_params: &mut optional,
                drop_params,
                catalog,
            },
            &self.costs,
        ))?;
        Ok(optional)
    }
}

impl CapabilityFixture {
    fn field(&self, model: &str, key: &str) -> Option<&Json> {
        match model.split_once('/') {
            Some((_, bare)) => self
                .entries
                .get(model)
                .and_then(|entry| entry.get(key))
                .or_else(|| self.entries.get(bare).and_then(|entry| entry.get(key))),
            None if self.resolvable.contains(model) => {
                self.entries.get(model).and_then(|entry| entry.get(key))
            }
            None => None,
        }
    }

    fn bool_field(&self, model: &str, key: &str) -> Option<bool> {
        match self.field(model, key) {
            Some(Json::Bool(value)) => Some(*value),
            Some(
                Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        }
    }

    fn text_field(&self, model: &str, key: &str) -> Option<String> {
        match self.field(model, key) {
            Some(Json::Text(value)) => Some(value.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        }
    }
}

impl ReasoningCapabilities for CapabilityFixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.bool_field(request.model, key) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(self.text_field(request.model, key))
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.bool_field(request.model, key) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(self.bool_field(request.model, "supports_response_schema") == Some(true))
    }

    fn catalogue_declares_default(&mut self) -> bool {
        let entries: Vec<String> = self
            .entries
            .values()
            .map(|entry| serde_json::to_string(entry).expect("a fixture entry"))
            .collect();
        catalogue_declares_default(entries.iter().map(String::as_str))
    }
}

pub(super) struct NoSources;

fn refuse<T>(kind: &str) -> Result<T, SourceFailure> {
    Err(SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {kind} source is configured"),
    })
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("client secret")
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("username and password")
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("SDK refresh")
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}

#[must_use]
pub(super) fn settings(token: Option<&str>, client_secret: Option<&str>) -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: token.map(SecretString::from),
        tenant_id: client_secret.map(|_| String::from("fixture-tenant")),
        client_id: client_secret.map(|_| String::from("fixture-client")),
        client_secret: client_secret.map(SecretString::from),
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}

pub(super) struct EnvironmentFixture<'data> {
    pub(super) headers: Headers,
    pub(super) argument_key: Option<&'data SecretString>,
    pub(super) params_key: Option<&'data SecretString>,
    pub(super) configured_key: Option<&'data SecretString>,
    pub(super) credentials: &'data DeploymentSettings,
    pub(super) sources: &'data dyn CredentialSources,
    pub(super) refresh_enabled: bool,
    pub(super) api_base: Option<&'data str>,
}

impl<'data> EnvironmentFixture<'data> {
    pub(super) fn request(&self) -> EnvironmentRequest<'data> {
        EnvironmentRequest {
            headers: self.headers.clone(),
            argument_key: self.argument_key,
            params_key: self.params_key,
            configured_key: self.configured_key,
            token: TokenSource {
                credentials: self.credentials,
                refresh_enabled: self.refresh_enabled,
                provider: None,
                sources: self.sources,
            },
            api_base: self.api_base,
        }
    }
}

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime")
}

#[derive(Clone, Copy, Default)]
pub(super) struct Keys<'data> {
    pub(super) argument: Option<&'data SecretString>,
    pub(super) params: Option<&'data SecretString>,
    pub(super) configured: Option<&'data SecretString>,
}

#[must_use]
pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    let mut built = Headers::new();
    for (name, value) in pairs {
        built.insert(String::from(*name), String::from(*value));
    }
    built
}

pub(super) fn validate(
    keys: Keys<'_>,
    credentials: &DeploymentSettings,
    api_base: Option<&str>,
) -> Headers {
    validate_with(Headers::new(), keys, credentials, api_base).expect("the fixture validates")
}

pub(super) fn validate_with(
    initial: Headers,
    keys: Keys<'_>,
    credentials: &DeploymentSettings,
    api_base: Option<&str>,
) -> Result<Headers, EnvironmentError> {
    validate_environment(
        EnvironmentFixture {
            headers: initial,
            argument_key: keys.argument,
            params_key: keys.params,
            configured_key: keys.configured,
            credentials,
            sources: &NoSources,
            refresh_enabled: false,
            api_base,
        }
        .request(),
    )
}
