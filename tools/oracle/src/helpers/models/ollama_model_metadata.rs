use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::ollama_model_metadata::{LiteLlmGlobals, OllamaModelInfo, OllamaSettings};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

use transport::{FixtureOutcome, FixtureTransport, success_reply};

mod tests;
mod transport;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    ConfiguredOriginAuth(OriginAuth),
    StaticCatalogFirst(StaticCatalog),
    RuntimeModelQuery(ModelQuery),
    RuntimeModelFields(ModelFields),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    globals: GlobalsInput,
    #[serde(default)]
    secret_manager_api_key: Option<String>,
    #[serde(default)]
    catalog: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalsInput {
    api_key: Option<String>,
    openai_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginAuth {
    model: String,
    groups: Vec<AuthGroup>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthGroup {
    fixture: Fixture,
    queries: Vec<AuthQuery>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthQuery {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StaticCatalog {
    fixture: Fixture,
    models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelQuery {
    fixture: Fixture,
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    scenarios: Vec<QueryScenario>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum QueryScenario {
    Success { body: String },
    Failure,
    Status { status: u16, body: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelFields {
    fixture: Fixture,
    model: String,
    bodies: Vec<String>,
}

struct LoadedFixture {
    settings: OllamaSettings,
    globals: LiteLlmGlobals,
    catalog: ModelCatalog,
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn load_fixture(fixture: &Fixture) -> Result<LoadedFixture, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: fixture.env.clone(),
    }
    .load("")
    .map_err(|error| helper_error("ConfigError", error.to_string()))?;
    let mut entries = json!({});
    for key in &fixture.catalog {
        entries[key.as_str()] = json!({});
    }
    let bytes = serde_json::to_vec(&entries)
        .map_err(|error| helper_error("Serialize", error.to_string()))?;
    let catalog = ModelCatalog::parse(&bytes, UnknownFields::Reject)
        .map_err(|error| helper_error("CatalogError", error.to_string()))?;
    Ok(LoadedFixture {
        settings: OllamaSettings {
            api_key: config.sections.ollama_model_metadata.api_key.clone(),
            api_base: config.sections.ollama_model_metadata.api_base.clone(),
        },
        globals: LiteLlmGlobals {
            api_key: fixture.globals.api_key.clone().map(SecretString::from),
            openai_key: fixture.globals.openai_key.clone().map(SecretString::from),
            secret_manager_api_key: fixture
                .secret_manager_api_key
                .clone()
                .map(SecretString::from),
        },
        catalog,
    })
}

fn origin_auth(input: &OriginAuth) -> HelperResult {
    let mut groups = Vec::new();
    for group in &input.groups {
        let fixture = load_fixture(&group.fixture)?;
        let mut observations = Vec::new();
        for query in &group.queries {
            let transport = success_reply();
            let resolver = OllamaModelInfo {
                settings: &fixture.settings,
                globals: &fixture.globals,
                catalog: &fixture.catalog,
                transport: &transport,
            };
            let api_key = query.api_key.clone().map(SecretString::from);
            resolver
                .get_runtime_model_info(&input.model, query.api_base.as_deref(), api_key.as_ref())
                .map_err(|error| helper_error("OllamaMetadataError", error.to_string()))?;
            let recorded = transport.only_request()?;
            observations.push(json!({
                "url": recorded.url,
                "authorization": recorded.authorization,
                "name": recorded.name,
            }));
        }
        groups.push(json!(observations));
    }
    Ok(json!(groups))
}

fn static_catalog(input: &StaticCatalog) -> HelperResult {
    let fixture = load_fixture(&input.fixture)?;
    let mut observations = Vec::new();
    for model in &input.models {
        let transport = success_reply();
        let resolver = OllamaModelInfo {
            settings: &fixture.settings,
            globals: &fixture.globals,
            catalog: &fixture.catalog,
            transport: &transport,
        };
        let info = resolver
            .get_model_info(model, None, None)
            .map_err(|error| helper_error("OllamaMetadataError", error.to_string()))?;
        observations.push(json!({
            "result": info,
            "calls": transport.recorded().len(),
        }));
    }
    Ok(json!(observations))
}

fn model_query(input: &ModelQuery) -> HelperResult {
    let fixture = load_fixture(&input.fixture)?;
    let api_key = input.api_key.clone().map(SecretString::from);
    let mut observations = Vec::new();
    for scenario in &input.scenarios {
        let outcome = match scenario {
            QueryScenario::Success { body } => FixtureOutcome::Reply {
                status: 200,
                body: body.as_bytes().to_vec(),
            },
            QueryScenario::Failure => FixtureOutcome::Failure,
            QueryScenario::Status { status, body } => FixtureOutcome::Reply {
                status: *status,
                body: body.as_bytes().to_vec(),
            },
        };
        let transport = FixtureTransport::new(outcome);
        let resolver = OllamaModelInfo {
            settings: &fixture.settings,
            globals: &fixture.globals,
            catalog: &fixture.catalog,
            transport: &transport,
        };
        let info = resolver
            .get_runtime_model_info(&input.model, input.api_base.as_deref(), api_key.as_ref())
            .map_err(|error| helper_error("OllamaMetadataError", error.to_string()))?;
        let recorded = transport.only_request()?;
        observations.push(json!({
            "url": recorded.url,
            "authorization": recorded.authorization,
            "name": recorded.name,
            "info": info,
        }));
    }
    Ok(json!(observations))
}

fn model_fields(input: &ModelFields) -> HelperResult {
    let fixture = load_fixture(&input.fixture)?;
    let mut observations = Vec::new();
    for body in &input.bodies {
        let transport = FixtureTransport::new(FixtureOutcome::Reply {
            status: 200,
            body: body.as_bytes().to_vec(),
        });
        let resolver = OllamaModelInfo {
            settings: &fixture.settings,
            globals: &fixture.globals,
            catalog: &fixture.catalog,
            transport: &transport,
        };
        let info = match resolver.get_runtime_model_info(&input.model, None, None) {
            Ok(info) => Some(info),
            Err(_) => None,
        };
        observations.push(json!({ "info": info }));
    }
    Ok(json!(observations))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ConfiguredOriginAuth(args) => origin_auth(args),
            Input::StaticCatalogFirst(args) => static_catalog(args),
            Input::RuntimeModelQuery(args) => model_query(args),
            Input::RuntimeModelFields(args) => model_fields(args),
        }
    }
}
