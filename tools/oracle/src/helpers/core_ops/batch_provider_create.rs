use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::batch_provider_create::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_create::error::BatchProviderCreateError;
use betterlite_core::api::batch_provider_create::transports::BatchCreateTransport;
use betterlite_core::api::batch_provider_create::{
    BatchProviderCreate, BatchProviderCreateCall, BatchProviderCreateInput, ProviderBatchCall,
    ProviderConfigCall, VertexCredentials,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    module: Module,
    steps: Vec<BatchProviderCreateInput>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Module {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
    api_version: Option<String>,
    azure_key: Option<SecretString>,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

fn settings(module: &Module, env: &BTreeMap<String, String>) -> Settings {
    let read = |name: &str| env.get(name).filter(|value| !value.is_empty()).cloned();
    Settings {
        api_base: module.api_base.clone().filter(|value| !value.is_empty()),
        organization: None,
        api_key: module.api_key.clone(),
        openai_key: module.openai_key.clone(),
        api_version: module.api_version.clone().filter(|value| !value.is_empty()),
        azure_key: module.azure_key.clone(),
        vertex_project: module
            .vertex_project
            .clone()
            .filter(|value| !value.is_empty()),
        vertex_location: module
            .vertex_location
            .clone()
            .filter(|value| !value.is_empty()),
        openai_base_url: read("OPENAI_BASE_URL"),
        openai_api_base: read("OPENAI_API_BASE"),
        openai_organization: read("OPENAI_ORGANIZATION"),
        openai_api_key: read("OPENAI_API_KEY").map(SecretString::from),
        azure_api_base: read("AZURE_API_BASE"),
        azure_api_version: read("AZURE_API_VERSION"),
        azure_openai_api_key: read("AZURE_OPENAI_API_KEY").map(SecretString::from),
        azure_api_key: read("AZURE_API_KEY").map(SecretString::from),
        azure_ad_token: read("AZURE_AD_TOKEN").map(SecretString::from),
        vertexai_project: read("VERTEXAI_PROJECT"),
        vertexai_location: read("VERTEXAI_LOCATION"),
        vertexai_credentials: read("VERTEXAI_CREDENTIALS").map(SecretString::from),
    }
}

struct ScriptedTransport;

impl BatchCreateTransport for ScriptedTransport {
    fn create(
        &self,
        _call: &BatchProviderCreateCall,
    ) -> Result<BatchObject, BatchProviderCreateError> {
        Ok(BatchObject {
            id: "batch-oracle".to_owned(),
            completion_window: "24h".to_owned(),
            created_at: 0,
            endpoint: "/v1/chat/completions".to_owned(),
            input_file_id: "file-oracle".to_owned(),
            object: BatchObjectKind::Batch,
            status: BatchStatus::Validating,
            cancelled_at: None,
            cancelling_at: None,
            completed_at: None,
            error_file_id: None,
            errors: None,
            expired_at: None,
            expires_at: None,
            failed_at: None,
            finalizing_at: None,
            in_progress_at: None,
            metadata: None,
            model: None,
            output_file_id: None,
            request_counts: None,
            usage: None,
            provider_fields: Map::new(),
        })
    }
}

fn pairs<'entries, Entries>(entries: Entries) -> Vec<Value>
where
    Entries: IntoIterator<Item = (&'entries String, &'entries String)>,
{
    let mut sorted: Vec<(&String, &String)> = entries.into_iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(right.0));
    sorted
        .into_iter()
        .map(|(key, value)| json!([key, value]))
        .collect()
}

fn body_json(body: &CreateBatchRequest) -> Value {
    json!({
        "completion_window": body.completion_window,
        "endpoint": body.endpoint,
        "input_file_id": body.input_file_id,
        "metadata": body.metadata.as_ref().map(pairs),
        "extra_headers": body.extra_headers.as_ref().map(pairs),
        "extra_body": body.extra_body.as_ref().map(pairs),
        "output_expires_after": body.output_expires_after,
    })
}

fn credentials_json(credentials: Option<&VertexCredentials>) -> Value {
    match credentials {
        None => Value::Null,
        Some(VertexCredentials::Text(text)) => json!(text.expose_secret()),
        Some(VertexCredentials::Fields(fields)) => {
            let mut sorted: Vec<(&String, &SecretString)> = fields.iter().collect();
            sorted.sort_by(|left, right| left.0.cmp(right.0));
            Value::Array(
                sorted
                    .into_iter()
                    .map(|(key, value)| json!([key, value.expose_secret()]))
                    .collect(),
            )
        }
    }
}

fn call_json(record: &BatchProviderCreateCall) -> Value {
    match record {
        BatchProviderCreateCall::ProviderConfig(call) => provider_config_json(call),
        BatchProviderCreateCall::Xai(call) => transport_json("xai", call, &[api_key_field(call)]),
        BatchProviderCreateCall::OpenAiCompatible(call) => transport_json(
            "openai_compatible",
            call,
            &[
                api_key_field(call),
                ("organization", json!(call.organization)),
                ("max_retries", json!(call.max_retries)),
            ],
        ),
        BatchProviderCreateCall::Azure(call) => transport_json(
            "azure",
            call,
            &[
                api_key_field(call),
                ("api_version", json!(call.api_version)),
                ("max_retries", json!(call.max_retries)),
            ],
        ),
        BatchProviderCreateCall::VertexAi(call) => transport_json(
            "vertex_ai",
            call,
            &[
                ("vertex_project", json!(call.vertex_project)),
                ("vertex_location", json!(call.vertex_location)),
                (
                    "vertex_credentials",
                    credentials_json(call.vertex_credentials.as_ref()),
                ),
                ("max_retries", json!(call.max_retries)),
                ("custom_endpoint", json!(call.custom_endpoint)),
            ],
        ),
    }
}

fn provider_config_json(call: &ProviderConfigCall) -> Value {
    json!({
        "branch": "provider_config",
        "provider": call.provider.as_str(),
        "model": call.model,
        "body": body_json(&call.body),
        "headers": pairs(&call.headers),
        "api_base": call.api_base,
        "api_key": call.api_key.as_ref().map(secrecy::ExposeSecret::expose_secret),
        "timeout": call.timeout,
        "is_async": call.is_async,
    })
}

// the vertex handler takes no api_key, so that key is recorded only by the branches that pass one
fn api_key_field(call: &ProviderBatchCall) -> (&'static str, Value) {
    let key = call
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    ("api_key", json!(key))
}

// the keys every transport branch records, plus the ones only its provider sends
fn transport_json(branch: &str, call: &ProviderBatchCall, extras: &[(&str, Value)]) -> Value {
    let mut fields = Map::new();
    fields.insert("branch".to_owned(), json!(branch));
    fields.insert("body".to_owned(), body_json(&call.body));
    fields.insert("api_base".to_owned(), json!(call.api_base));
    fields.insert("timeout".to_owned(), json!(call.timeout));
    fields.insert("is_async".to_owned(), json!(call.is_async));
    for (key, value) in extras {
        fields.insert((*key).to_owned(), value.clone());
    }
    Value::Object(fields)
}

fn step_outcome(
    service: &BatchProviderCreate<'_, ScriptedTransport>,
    input: &BatchProviderCreateInput,
) -> Value {
    match service.create(input) {
        Ok(result) => call_json(&result.call),
        Err(error) => json!({ "raised": error.status_code() }),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let catalog = ModelCatalog::parse(b"{}", UnknownFields::Reject).or_config_error()?;
        let names = BTreeSet::new();
        let environment = BTreeMap::new();
        let context = ResolutionContext {
            catalog: &catalog,
            known_model_names: &names,
            custom_providers: &names,
            environment: &environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let service = BatchProviderCreate {
            settings: settings(&input.module, &input.env),
            resolution: &context,
            transport: ScriptedTransport,
            limits: ApiLimits::default(),
        };
        Ok(Value::Array(
            input
                .steps
                .iter()
                .map(|step| step_outcome(&service, step))
                .collect(),
        ))
    }
}
