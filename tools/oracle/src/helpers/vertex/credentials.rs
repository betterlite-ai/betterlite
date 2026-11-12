use std::collections::BTreeMap;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_vertex::credential_sources::getters::{
    VertexParams, explicit_vertex_ai_location, safe_get_vertex_ai_credentials,
    safe_get_vertex_ai_location, safe_get_vertex_ai_project,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize, Default)]
pub(crate) struct Input {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize, Default)]
struct Scenario {
    #[serde(default)]
    params: Params,
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct Params {
    #[serde(rename = "vertex_project")]
    project: Option<String>,
    #[serde(rename = "vertex_ai_project")]
    ai_project: Option<String>,
    #[serde(rename = "vertex_location")]
    location: Option<String>,
    #[serde(rename = "vertex_ai_location")]
    ai_location: Option<String>,
    #[serde(rename = "vertex_credentials")]
    credentials: Option<String>,
    #[serde(rename = "vertex_ai_credentials")]
    ai_credentials: Option<String>,
}

#[derive(Deserialize, Default)]
struct Settings {
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut scenarios = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            scenarios.push(projection(scenario));
        }
        Ok(json!({ "scenarios": scenarios }))
    }
}

fn projection(scenario: &Scenario) -> Value {
    let settings = resolved_settings(scenario);
    let params_credentials = scenario
        .params
        .credentials
        .as_deref()
        .map(|text| ConfiguredCredentials::Text(SecretString::from(text)));
    let params_ai_credentials = scenario
        .params
        .ai_credentials
        .as_deref()
        .map(|text| ConfiguredCredentials::Text(SecretString::from(text)));
    let params = VertexParams {
        project: scenario.params.project.as_deref(),
        ai_project: scenario.params.ai_project.as_deref(),
        location: scenario.params.location.as_deref(),
        ai_location: scenario.params.ai_location.as_deref(),
        credentials: params_credentials.as_ref(),
        ai_credentials: params_ai_credentials.as_ref(),
    };
    json!({
        "project": safe_get_vertex_ai_project(&params, &settings),
        "location": safe_get_vertex_ai_location(&params, &settings),
        "credentials": safe_get_vertex_ai_credentials(&params, &settings).and_then(ConfiguredCredentials::as_text),
        "explicit_location": explicit_vertex_ai_location(&params),
    })
}

// the case carries the YAML settings and environment the loader would resolve at startup
fn resolved_settings(scenario: &Scenario) -> VertexSettings {
    let read = |name: &str| {
        scenario
            .environment
            .get(name)
            .filter(|text| !text.is_empty())
            .cloned()
    };
    let credentials = read("VERTEXAI_CREDENTIALS")
        .map(|text| ConfiguredCredentials::Text(SecretString::from(text)));
    VertexSettings {
        project: scenario
            .settings
            .vertex_project
            .clone()
            .or_else(|| read("VERTEXAI_PROJECT")),
        location: scenario
            .settings
            .vertex_location
            .clone()
            .or_else(|| read("VERTEXAI_LOCATION"))
            .or_else(|| read("VERTEX_LOCATION")),
        credentials,
        adc: AdcEnvironment::default(),
    }
}
