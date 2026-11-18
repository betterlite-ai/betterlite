#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_search::{
    CONNECTION_ID_ENV, MODEL_ENV, PROJECT_ENDPOINT_ENV, Settings, TOKEN_ENV,
};

#[derive(Clone, Copy)]
enum Field {
    ProjectEndpoint,
    Model,
    ConnectionId,
    Token,
}

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_search
}

fn read(settings: &Settings, field: Field) -> Option<String> {
    match field {
        Field::ProjectEndpoint => settings.project_endpoint.clone(),
        Field::Model => settings.model.clone(),
        Field::ConnectionId => settings.connection_id.clone(),
        Field::Token => settings
            .token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
    }
}

struct Case {
    name: &'static str,
    value: &'static str,
    expected: &'static str,
    field: Field,
    secret: bool,
}

enum Scenario {
    Load(Case),
    UnrelatedKey,
}

#[rstest]
#[case::loads_bing_grounding_project_endpoint(Scenario::Load(Case {
    name: PROJECT_ENDPOINT_ENV,
    value: "https://project.services.ai.azure.com/api/projects/example",
    expected: "https://project.services.ai.azure.com/api/projects/example",
    field: Field::ProjectEndpoint,
    secret: false,
}))]
#[case::loads_bing_grounding_model(Scenario::Load(Case {
    name: MODEL_ENV,
    value: "gpt-4.1",
    expected: "gpt-4.1",
    field: Field::Model,
    secret: false,
}))]
#[case::loads_bing_grounding_connection_id(Scenario::Load(Case {
    name: CONNECTION_ID_ENV,
    value: "connection-1",
    expected: "connection-1",
    field: Field::ConnectionId,
    secret: false,
}))]
#[case::loads_bing_grounding_token(Scenario::Load(Case {
    name: TOKEN_ENV,
    value: "fixture-bing-token",
    expected: "fixture-bing-token",
    field: Field::Token,
    secret: true,
}))]
#[case::rejects_unrelated_key(Scenario::UnrelatedKey)]
fn azure_search_config_cases(#[case] scenario: Scenario) {
    let case = match scenario {
        Scenario::Load(case) => case,
        Scenario::UnrelatedKey => {
            rejects_unrelated_key();
            return;
        }
    };
    let explicit = settings(&[(case.name, case.value)]);
    assert_eq!(read(&explicit, case.field).as_deref(), Some(case.expected));
    assert_eq!(read(&settings(&[]), case.field).as_deref(), None);
    assert_eq!(
        read(&settings(&[(case.name, "")]), case.field).as_deref(),
        None
    );
    assert_eq!(
        read(&settings(&[("UNRELATED", case.value)]), case.field).as_deref(),
        None
    );
    if case.secret {
        assert!(!format!("{explicit:?}").contains(case.value));
    }
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  azure_search_unrelated_key: 1\n";
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_search_unrelated_key".to_owned()]
    );
}
