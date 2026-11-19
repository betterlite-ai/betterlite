use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::assistant_records::error::AssistantCreateFailure;
use betterlite_core::api::assistant_records::service::AssistantRecords;
use betterlite_core::api::assistant_records::{
    AssistantCreateInput, AsyncFlag, ClientClass, CreateTimeout,
};
use betterlite_error::envelope::WireField;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::assistants::records::{
    AssistantObject, AssistantTool, ResponseFormat, ToolResources,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use serde_with::rust::double_option;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::result::{Failure, Nullable};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    #[default]
    Sync,
    Async,
}

// "mock" hands the mock origin over as the provider base; "absent" leaves the base to the environment
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiBase {
    #[default]
    Mock,
    Absent,
}

// The SDK-visible create_assistants arguments; acreate_assistants is the same call with Entry::Async.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub entry: Entry,
    #[serde(default)]
    pub api_base: ApiBase,
    pub api_key: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<CreateTimeout>,
    pub request_timeout: Option<f64>,
    pub azure_ad_token: Option<String>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub azure_username: Option<String>,
    pub azure_password: Option<String>,
    pub azure_scope: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub tools: Option<Vec<AssistantTool>>,
    pub tool_resources: Option<ToolResources>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub response_format: Option<ResponseFormat>,
    pub client: Option<ClientClass>,
    pub async_create_assistants: Option<AsyncFlag>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub extra_body: Option<Option<BTreeMap<String, String>>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

// The provider answer, or the exception the LiteLLM façade raised, with the fields it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantView {
    pub id: String,
    pub object: String,
    pub model: String,
    pub created_at: i64,
    pub name: Option<String>,
    pub tools: Vec<Value>,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        send(self, api_base, env)
    }
}

async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let input = input(call, api_base);
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let config = loader
        .load("{}")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let catalog = ModelCatalog::default();
    let names = std::collections::BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let sources = NoSources;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let transport =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    let service = AssistantRecords {
        settings: config.sections.resource_sdk_settings.clone(),
        azure: config.sections.azure_endpoints.clone(),
        environment: env,
        sources: &sources,
        resolution: &resolution,
        redactor: &redactor,
        transport: Arc::new(transport),
    };
    let result = match call.entry {
        Entry::Sync => service.create_assistants(&input).await,
        Entry::Async => service.acreate_assistants(&input).await,
    };
    let outcome = match result {
        Ok(result) => Outcome {
            assistant: Some(view(result.assistant)?),
            failure: None,
        },
        Err(failure) => Outcome {
            assistant: None,
            failure: Some(record(failure)?),
        },
    };
    Ok(SdkResult::AssistantCreate {
        value: Box::new(outcome),
    })
}

fn input(call: &Call, api_base: &str) -> AssistantCreateInput {
    AssistantCreateInput {
        provider: call.provider.clone(),
        model: call.model.clone(),
        name: call.name.clone(),
        description: call.description.clone(),
        instructions: call.instructions.clone(),
        tools: call.tools.clone(),
        tool_resources: call.tool_resources.clone(),
        metadata: call.metadata.clone(),
        temperature: call.temperature,
        top_p: call.top_p,
        response_format: call.response_format.clone(),
        client: call.client,
        api_key: call.api_key.clone().map(SecretString::from),
        api_base: match call.api_base {
            ApiBase::Mock => Some(api_base.to_owned()),
            ApiBase::Absent => None,
        },
        api_version: call.api_version.clone(),
        organization: call.organization.clone(),
        max_retries: call.max_retries,
        timeout: call.timeout.clone(),
        request_timeout: call.request_timeout,
        async_create_assistants: call.async_create_assistants.clone(),
        azure_ad_token: call.azure_ad_token.clone().map(SecretString::from),
        tenant_id: call.tenant_id.clone(),
        client_id: call.client_id.clone(),
        client_secret: call.client_secret.clone().map(SecretString::from),
        azure_username: call.azure_username.clone(),
        azure_password: call.azure_password.clone().map(SecretString::from),
        azure_scope: call.azure_scope.clone(),
        extra_body: call.extra_body.clone(),
        extra: call.extra.clone(),
    }
}

fn view(assistant: AssistantObject) -> Result<AssistantView, SdkHttpError> {
    let tools = assistant
        .tools
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<Value>, _>>()
        .map_err(SdkHttpError::Serialize)?;
    Ok(AssistantView {
        id: assistant.id,
        object: assistant.object,
        model: assistant.model,
        created_at: assistant.created_at,
        name: assistant.name,
        tools,
    })
}

fn record(failure: AssistantCreateFailure) -> Result<Failure, SdkHttpError> {
    let Some(status) = failure.status else {
        return Err(SdkHttpError::Invalid(format!(
            "the wrapper raised {} without a status code",
            failure.class
        )));
    };
    Ok(Failure {
        class: failure.class,
        status,
        kind: nullable(failure.kind),
        param: nullable(failure.param),
        code: nullable(failure.code),
        message: failure.message,
    })
}

fn nullable(field: WireField) -> Nullable {
    match field {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
