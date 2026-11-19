use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, WorkloadIdentity, get_complete_url,
    validate_environment,
};
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::{
    ListInputItemsQuery, ResourceOperation, ResourceRequest, list_input_items_request,
    resource_request,
};
use betterlite_openai::response_resources::response::{TypedReply, json_body, response_reply};
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use secrecy::SecretString;

// The litellm_params fields the four handlers read
#[derive(Debug, Clone, Default)]
pub struct ReadParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The OPENAI_* values get_secret_str reads, resolved by the config section
#[derive(Debug, Clone, Default)]
pub struct ReadEnvironment {
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub api_base: Option<String>,
}

// The URL and query a transformer builds, in the JSON type the pre-call record carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResource {
    pub url: String,
    pub query: IndexMap<String, Json>,
}

// The provider transform a read calls: retrieve uses only the base, input items also carries the query
#[derive(Debug)]
pub enum ReadOperation<'query> {
    Retrieve,
    InputItems(&'query ListInputItemsQuery),
}

// The BaseResponsesAPIConfig members the four handlers call; B933 adapts a provider config to this
pub trait ReadConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: BTreeMap<String, String>,
        params: &ReadParams,
    ) -> BTreeMap<String, String>;

    fn complete_url(&self, params: &ReadParams) -> String;

    fn read_resource(
        &self,
        response_id: &str,
        api_base: &str,
        params: &ReadParams,
        operation: ReadOperation<'_>,
    ) -> Result<ReadResource, ResponseResourceError>;

    fn retrieve_reply(&self, reply: &RawResponse<'_>) -> Result<TypedReply, ResponseResourceError>;

    fn input_items_reply(&self, reply: &RawResponse<'_>) -> Result<Json, ResponseResourceError>;
}

// OpenAI's transforms over B509 and B508, whose validation rewrites Authorization and Content-Type in the map it is given
pub struct OpenAiReadConfig<Identity> {
    globals: LiteLlmGlobals,
    environment: ReadEnvironment,
    identity: Mutex<Identity>,
}

impl<Identity> OpenAiReadConfig<Identity> {
    #[must_use]
    pub const fn new(
        globals: LiteLlmGlobals,
        environment: ReadEnvironment,
        identity: Identity,
    ) -> Self {
        Self {
            globals,
            environment,
            identity: Mutex::new(identity),
        }
    }
}

impl<Identity> ReadConfig for OpenAiReadConfig<Identity>
where
    Identity: WorkloadIdentity + Send,
{
    fn validate_environment(
        &self,
        headers: BTreeMap<String, String>,
        params: &ReadParams,
    ) -> BTreeMap<String, String> {
        let mut identity = self.identity.lock().unwrap_or_else(PoisonError::into_inner);
        validate_environment(
            headers,
            ApiKeySources {
                litellm_params: params.api_key.as_ref(),
                litellm: self.globals.api_key.as_ref(),
                openai: self.globals.openai_key.as_ref(),
                environment: self.environment.api_key.as_ref(),
            },
            params.api_base.as_deref(),
            ResponsesProvider::OpenAi,
            &mut identity,
        )
    }

    fn complete_url(&self, params: &ReadParams) -> String {
        get_complete_url(
            params.api_base.as_deref(),
            self.globals.api_base.as_deref(),
            self.environment.base_url.as_deref(),
            self.environment.api_base.as_deref(),
        )
    }

    fn read_resource(
        &self,
        response_id: &str,
        api_base: &str,
        _params: &ReadParams,
        operation: ReadOperation<'_>,
    ) -> Result<ReadResource, ResponseResourceError> {
        match operation {
            ReadOperation::Retrieve => {
                resource_request(response_id, api_base, ResourceOperation::Get).map(resource)
            }
            ReadOperation::InputItems(query) => {
                list_input_items_request(response_id, api_base, query).map(resource)
            }
        }
    }

    fn retrieve_reply(&self, reply: &RawResponse<'_>) -> Result<TypedReply, ResponseResourceError> {
        response_reply(reply)
    }

    fn input_items_reply(&self, reply: &RawResponse<'_>) -> Result<Json, ResponseResourceError> {
        json_body(reply)
    }
}

fn resource(request: ResourceRequest) -> ReadResource {
    ReadResource {
        url: request.url,
        query: request
            .payload
            .iter()
            .map(|(name, value)| (name.clone(), json(value)))
            .collect(),
    }
}

fn json(value: &PromptJson) -> Json {
    match value {
        PromptJson::Null => Json::Null,
        PromptJson::Bool(flag) => Json::Bool(*flag),
        PromptJson::Number(number) => Json::Number(number.clone()),
        PromptJson::Text(text) => Json::Text(text.clone()),
        PromptJson::List(items) => Json::List(items.iter().map(json).collect()),
        PromptJson::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), json(item)))
                .collect(),
        ),
    }
}

// The seam B501 fills; a caller with no workload identity passes this
#[derive(Debug, Clone, Copy, Default)]
pub struct NoWorkloadIdentity;

impl WorkloadIdentity for NoWorkloadIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Option<SecretString> {
        None
    }
}
