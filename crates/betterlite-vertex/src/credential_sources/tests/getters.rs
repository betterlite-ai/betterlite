#![cfg(test)]

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use secrecy::SecretString;

use crate::credential_sources::getters::{
    VertexParams, explicit_vertex_ai_location, safe_get_vertex_ai_credentials,
    safe_get_vertex_ai_location, safe_get_vertex_ai_project,
};

fn text(value: &str) -> ConfiguredCredentials {
    ConfiguredCredentials::Text(SecretString::from(value))
}

fn params<'values>(
    project: Option<&'values str>,
    ai_project: Option<&'values str>,
    location: Option<&'values str>,
    ai_location: Option<&'values str>,
) -> VertexParams<'values> {
    VertexParams {
        project,
        ai_project,
        location,
        ai_location,
        credentials: None,
        ai_credentials: None,
    }
}

fn settings(project: Option<&str>, location: Option<&str>) -> VertexSettings {
    VertexSettings {
        project: project.map(str::to_owned),
        location: location.map(str::to_owned),
        credentials: None,
        adc: AdcEnvironment::default(),
    }
}

fn empty() -> VertexParams<'static> {
    params(None, None, None, None)
}

#[test]
fn resolves_the_project_aliases_in_order() {
    let module = settings(Some("module-project"), None);
    assert_eq!(
        safe_get_vertex_ai_project(
            &params(Some("project"), Some("alias-project"), None, None),
            &module
        ),
        Some("project")
    );
    assert_eq!(
        safe_get_vertex_ai_project(&params(None, Some("alias-project"), None, None), &module),
        Some("alias-project")
    );
    assert_eq!(
        safe_get_vertex_ai_project(&empty(), &module),
        Some("module-project")
    );
    assert_eq!(
        safe_get_vertex_ai_project(&empty(), &settings(None, None)),
        None
    );
}

#[test]
fn keeps_an_explicit_empty_project() {
    let module = settings(Some("module-project"), None);
    assert_eq!(
        safe_get_vertex_ai_project(&params(Some(""), None, None, None), &module),
        Some(""),
        "an explicit empty project is used as given"
    );
    assert_eq!(
        safe_get_vertex_ai_project(&empty(), &settings(Some(""), None)),
        Some("")
    );
}

#[test]
fn resolves_the_credential_aliases_in_order() {
    let params_credentials = text("params");
    let alias_credentials = text("alias");
    let mut module = settings(Some("module-project"), None);
    module.credentials = Some(text("settings"));
    let both = VertexParams {
        project: None,
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: Some(&params_credentials),
        ai_credentials: Some(&alias_credentials),
    };
    assert_eq!(
        safe_get_vertex_ai_credentials(&both, &module).and_then(ConfiguredCredentials::as_text),
        Some("params")
    );
    let alias = VertexParams {
        project: None,
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: Some(&alias_credentials),
    };
    assert_eq!(
        safe_get_vertex_ai_credentials(&alias, &module).and_then(ConfiguredCredentials::as_text),
        Some("alias")
    );
    assert_eq!(
        safe_get_vertex_ai_credentials(&empty(), &module).and_then(ConfiguredCredentials::as_text),
        Some("settings")
    );
    assert!(safe_get_vertex_ai_credentials(&empty(), &settings(None, None)).is_none());
}

#[test]
fn resolves_the_location_aliases_in_order() {
    let module = settings(None, Some("module-location"));
    assert_eq!(
        safe_get_vertex_ai_location(
            &params(None, None, Some("location"), Some("alias")),
            &module
        ),
        Some("location")
    );
    assert_eq!(
        safe_get_vertex_ai_location(&params(None, None, None, Some("alias")), &module),
        Some("alias")
    );
    assert_eq!(
        safe_get_vertex_ai_location(&empty(), &module),
        Some("module-location")
    );
    assert_eq!(
        safe_get_vertex_ai_location(&empty(), &settings(None, None)),
        None
    );
    assert_eq!(
        safe_get_vertex_ai_location(&params(None, None, Some(""), Some("alias")), &module),
        Some("alias")
    );
}

#[test]
fn explicit_location_has_no_fallback() {
    assert_eq!(
        explicit_vertex_ai_location(&params(None, None, Some("location"), Some("alias"))),
        Some("location")
    );
    assert_eq!(
        explicit_vertex_ai_location(&params(None, None, None, Some("alias"))),
        Some("alias")
    );
    assert_eq!(explicit_vertex_ai_location(&empty()), None);
    assert_eq!(
        explicit_vertex_ai_location(&params(None, None, Some(""), Some("alias"))),
        Some("alias")
    );
    assert_eq!(
        safe_get_vertex_ai_location(&empty(), &settings(None, Some("module-location"))),
        Some("module-location")
    );
    assert_eq!(
        explicit_vertex_ai_location(&empty()),
        None,
        "the explicit lookup never reaches the module setting"
    );
}
