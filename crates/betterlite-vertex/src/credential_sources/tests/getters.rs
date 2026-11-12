#![cfg(test)]

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use secrecy::SecretString;

use crate::credential_sources::getters::VertexParams;

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
        params(Some("project"), Some("alias-project"), None, None)
            .safe_get_vertex_ai_project(&module),
        Some("project")
    );
    assert_eq!(
        params(None, Some("alias-project"), None, None).safe_get_vertex_ai_project(&module),
        Some("alias-project")
    );
    assert_eq!(
        empty().safe_get_vertex_ai_project(&module),
        Some("module-project")
    );
    assert_eq!(
        empty().safe_get_vertex_ai_project(&settings(None, None)),
        None
    );
}

#[test]
fn keeps_an_explicit_empty_project() {
    let module = settings(Some("module-project"), None);
    assert_eq!(
        params(Some(""), None, None, None).safe_get_vertex_ai_project(&module),
        Some(""),
        "an explicit empty project is used as given"
    );
    assert_eq!(
        empty().safe_get_vertex_ai_project(&settings(Some(""), None)),
        Some("")
    );
}

#[test]
fn resolves_the_credential_aliases_in_order() {
    let params_credentials = text("params");
    let alias_credentials = text("alias");
    let mut module = settings(Some("module-project"), None);
    module.credentials = Some(text("settings"));
    let mut both = empty();
    both.credentials = Some(&params_credentials);
    both.ai_credentials = Some(&alias_credentials);
    assert_eq!(
        both.safe_get_vertex_ai_credentials(&module)
            .and_then(ConfiguredCredentials::as_text),
        Some("params")
    );
    let mut alias = empty();
    alias.ai_credentials = Some(&alias_credentials);
    assert_eq!(
        alias
            .safe_get_vertex_ai_credentials(&module)
            .and_then(ConfiguredCredentials::as_text),
        Some("alias")
    );
    assert_eq!(
        empty()
            .safe_get_vertex_ai_credentials(&module)
            .and_then(ConfiguredCredentials::as_text),
        Some("settings")
    );
    assert!(
        empty()
            .safe_get_vertex_ai_credentials(&settings(None, None))
            .is_none()
    );
}

#[test]
fn resolves_the_location_aliases_in_order() {
    let module = settings(None, Some("module-location"));
    assert_eq!(
        params(None, None, Some("location"), Some("alias")).safe_get_vertex_ai_location(&module),
        Some("location")
    );
    assert_eq!(
        params(None, None, None, Some("alias")).safe_get_vertex_ai_location(&module),
        Some("alias")
    );
    assert_eq!(
        empty().safe_get_vertex_ai_location(&module),
        Some("module-location")
    );
    assert_eq!(
        empty().safe_get_vertex_ai_location(&settings(None, None)),
        None
    );
    assert_eq!(
        params(None, None, Some(""), Some("alias")).safe_get_vertex_ai_location(&module),
        Some("alias")
    );
}

#[test]
fn explicit_location_has_no_fallback() {
    assert_eq!(
        params(None, None, Some("location"), Some("alias")).explicit_vertex_ai_location(),
        Some("location")
    );
    assert_eq!(
        params(None, None, None, Some("alias")).explicit_vertex_ai_location(),
        Some("alias")
    );
    assert_eq!(empty().explicit_vertex_ai_location(), None);
    assert_eq!(
        params(None, None, Some(""), Some("alias")).explicit_vertex_ai_location(),
        Some("alias")
    );
    assert_eq!(
        empty().safe_get_vertex_ai_location(&settings(None, Some("module-location"))),
        Some("module-location")
    );
    assert_eq!(
        empty().explicit_vertex_ai_location(),
        None,
        "the explicit lookup never reaches the module setting"
    );
}
