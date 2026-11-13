use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::ollama_model_metadata::reply::OllamaMetadataError;
use betterlite_models::ollama_model_metadata::{
    LiteLlmGlobals, OllamaModelInfo, OllamaSettings, RuntimeModelInfo,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

use transport::{FixtureOutcome, FixtureTransport, reply_outcome};

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
    let Some(object) = entries.as_object_mut() else {
        return Err(helper_error(
            "Fixture",
            "catalog entries must be an object".to_owned(),
        ));
    };
    for key in &fixture.catalog {
        object.insert(key.clone(), json!({}));
    }
    let bytes = serde_json::to_vec(&entries)
        .map_err(|error| helper_error("Serialize", error.to_string()))?;
    let catalog = ModelCatalog::parse(&bytes, UnknownFields::Reject)
        .map_err(|error| helper_error("CatalogError", error.to_string()))?;
    Ok(LoadedFixture {
        settings: OllamaSettings {
            api_key: config.sections.ollama_model_metadata.api_key.clone(),
            api_base: config.sections.ollama_model_metadata.api_base,
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

struct Observation {
    url: String,
    authorization: Option<String>,
    name: String,
    info: Result<RuntimeModelInfo, OllamaMetadataError>,
}

fn observe(
    fixture: &LoadedFixture,
    model: &str,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    outcome: FixtureOutcome,
) -> Result<Observation, Box<HelperError>> {
    let transport = FixtureTransport::new(outcome);
    let resolver = OllamaModelInfo {
        settings: &fixture.settings,
        globals: &fixture.globals,
        catalog: &fixture.catalog,
        transport: &transport,
    };
    let info = resolver.get_runtime_model_info(model, api_base, api_key);
    let recorded = transport.only_request()?;
    Ok(Observation {
        url: recorded.url,
        authorization: recorded.authorization,
        name: recorded.name,
        info,
    })
}

fn scenario_outcome(scenario: &QueryScenario) -> FixtureOutcome {
    match scenario {
        QueryScenario::Success { body } => reply_outcome(200, body),
        QueryScenario::Failure => FixtureOutcome::Failure,
        QueryScenario::Status { status, body } => reply_outcome(*status, body),
    }
}

fn origin_auth(input: &OriginAuth) -> HelperResult {
    let mut groups = Vec::new();
    for group in &input.groups {
        let fixture = load_fixture(&group.fixture)?;
        let mut observations = Vec::new();
        for query in &group.queries {
            let api_key = query.api_key.clone().map(SecretString::from);
            let observation = observe(
                &fixture,
                &input.model,
                query.api_base.as_deref(),
                api_key.as_ref(),
                reply_outcome(200, "{}"),
            )?;
            observation
                .info
                .map_err(|error| helper_error("OllamaMetadataError", error.to_string()))?;
            observations.push(json!({
                "url": observation.url,
                "authorization": observation.authorization,
                "name": observation.name,
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
        let transport = FixtureTransport::new(reply_outcome(200, "{}"));
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
        let observation = observe(
            &fixture,
            &input.model,
            input.api_base.as_deref(),
            api_key.as_ref(),
            scenario_outcome(scenario),
        )?;
        let info = observation
            .info
            .map_err(|error| helper_error("OllamaMetadataError", error.to_string()))?;
        observations.push(json!({
            "url": observation.url,
            "authorization": observation.authorization,
            "name": observation.name,
            "info": info,
        }));
    }
    Ok(json!(observations))
}

fn model_fields(input: &ModelFields) -> HelperResult {
    let fixture = load_fixture(&input.fixture)?;
    let mut observations = Vec::new();
    for body in &input.bodies {
        let observation = observe(&fixture, &input.model, None, None, reply_outcome(200, body))?;
        observations.push(json!({ "info": observation.info.ok() }));
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
