use rstest::rstest;

use crate::openapi_tools::document::{Components, PathItem};
use crate::openapi_tools::parameters::{
    ExtractedParameters, extract_parameters, resolve_operation_params,
};

use super::support::operation;

fn components(json: &str) -> Components {
    serde_json::from_str(json).unwrap()
}

fn path_item(json: &str) -> PathItem {
    serde_json::from_str(json).unwrap()
}

fn names(
    parameters: &[crate::openapi_tools::document::Parameter],
) -> Vec<(String, Option<String>)> {
    parameters
        .iter()
        .map(|param| {
            (
                param.name.clone().unwrap_or_default(),
                param.location.clone(),
            )
        })
        .collect()
}

#[test]
fn resolves_a_component_reference() {
    let resolved = resolve_operation_params(
        &operation(r##"{"parameters":[{"$ref":"#/components/parameters/limit"}]}"##),
        &path_item("{}"),
        &components(r#"{"parameters":{"limit":{"name":"limit","in":"query"}}}"#),
    );
    assert_eq!(
        names(&resolved.parameters),
        vec![("limit".to_owned(), Some("query".to_owned()))]
    );
}

#[test]
fn drops_a_reference_with_no_component_target() {
    let resolved = resolve_operation_params(
        &operation(r##"{"parameters":[{"$ref":"#/components/parameters/missing"}]}"##),
        &path_item("{}"),
        &components("{}"),
    );
    assert!(resolved.parameters.is_empty());
}

#[test]
fn drops_a_resolved_parameter_without_a_name() {
    let resolved = resolve_operation_params(
        &operation(
            r##"{"parameters":[{"$ref":"#/components/parameters/nameless"},{"name":"kept"}]}"##,
        ),
        &path_item("{}"),
        &components(r#"{"parameters":{"nameless":{"in":"query"}}}"#),
    );
    assert_eq!(resolved.parameters.len(), 1);
    assert_eq!(resolved.parameters[0].name.as_deref(), Some("kept"));
}

#[test]
fn drops_a_resolved_parameter_with_an_empty_name() {
    let resolved = resolve_operation_params(
        &operation(r##"{"parameters":[{"$ref":"#/components/parameters/empty"}]}"##),
        &path_item("{}"),
        &components(r#"{"parameters":{"empty":{"name":"","in":"query"}}}"#),
    );
    assert!(resolved.parameters.is_empty());
}

#[test]
fn keeps_a_ref_to_another_section_when_it_has_a_name() {
    let resolved = resolve_operation_params(
        &operation(r##"{"parameters":[{"$ref":"#/definitions/thing","name":"thing"}]}"##),
        &path_item("{}"),
        &components("{}"),
    );
    assert_eq!(resolved.parameters.len(), 1);
    assert_eq!(resolved.parameters[0].name.as_deref(), Some("thing"));
    assert_eq!(
        resolved.parameters[0].reference.as_deref(),
        Some("#/definitions/thing")
    );
}

#[test]
fn drops_a_ref_to_another_section_without_a_name() {
    let resolved = resolve_operation_params(
        &operation(r##"{"parameters":[{"$ref":"#/definitions/thing"}]}"##),
        &path_item("{}"),
        &components("{}"),
    );
    assert!(resolved.parameters.is_empty());
}

#[test]
fn operation_parameters_win_on_name_and_location() {
    let resolved = resolve_operation_params(
        &operation(r#"{"parameters":[{"name":"owner","in":"query","description":"op"}]}"#),
        &path_item(
            r#"{"parameters":[{"name":"owner","in":"query","description":"path"},{"name":"repo","in":"path"}]}"#,
        ),
        &components("{}"),
    );
    assert_eq!(
        resolved
            .parameters
            .iter()
            .map(|param| param.name.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("repo"), Some("owner")]
    );
    assert_eq!(resolved.parameters[1].description.as_deref(), Some("op"));
}

#[test]
fn operation_parameters_keep_their_order_after_the_path_level_ones() {
    let resolved = resolve_operation_params(
        &operation(r#"{"parameters":[{"name":"b","in":"query"},{"name":"c","in":"query"}]}"#),
        &path_item(r#"{"parameters":[{"name":"a","in":"query"}]}"#),
        &components("{}"),
    );
    assert_eq!(
        resolved
            .parameters
            .iter()
            .map(|param| param.name.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

#[test]
fn the_same_name_with_another_location_stays_twice() {
    let resolved = resolve_operation_params(
        &operation(r#"{"parameters":[{"name":"id","in":"path"}]}"#),
        &path_item(r#"{"parameters":[{"name":"id","in":"query"}]}"#),
        &components("{}"),
    );
    assert_eq!(resolved.parameters.len(), 2);
}

#[test]
fn an_operation_without_parameters_gets_an_empty_list() {
    let resolved = resolve_operation_params(
        &operation(r#"{"operationId":"x"}"#),
        &path_item("{}"),
        &components("{}"),
    );
    assert!(resolved.parameters.is_empty());
    assert_eq!(resolved.operation_id.as_deref(), Some("x"));
}

#[rstest]
#[case::empty(r#"{}"#, ExtractedParameters::default())]
#[case::all_three(
    r#"{"parameters":[{"name":"id","in":"path"},{"name":"q","in":"query"},{"name":"payload","in":"body"}]}"#,
    ExtractedParameters {
        path: vec!["id".to_owned()],
        query: vec!["q".to_owned()],
        body: vec!["payload".to_owned()],
    }
)]
#[case::header_and_cookie_are_ignored(
    r#"{"parameters":[{"name":"token","in":"header"},{"name":"session","in":"cookie"},{"name":"bare"}]}"#,
    ExtractedParameters::default()
)]
#[case::nameless_parameters_are_skipped(
    r#"{"parameters":[{"in":"path"},{"name":"id","in":"path"}]}"#,
    ExtractedParameters {
        path: vec!["id".to_owned()],
        query: Vec::new(),
        body: Vec::new(),
    }
)]
#[case::an_empty_name_is_a_name(
    r#"{"parameters":[{"name":"","in":"query"}]}"#,
    ExtractedParameters {
        path: Vec::new(),
        query: vec![String::new()],
        body: Vec::new(),
    }
)]
#[case::an_empty_request_body_appends_body(
    r#"{"requestBody":{}}"#,
    ExtractedParameters {
        path: Vec::new(),
        query: Vec::new(),
        body: vec!["body".to_owned()],
    }
)]
#[case::a_body_parameter_and_a_request_body_keep_both(
    r#"{"parameters":[{"name":"pet","in":"body"}],"requestBody":{}}"#,
    ExtractedParameters {
        path: Vec::new(),
        query: Vec::new(),
        body: vec!["pet".to_owned(), "body".to_owned()],
    }
)]
fn extracts_parameter_names(#[case] operation_json: &str, #[case] expected: ExtractedParameters) {
    assert_eq!(extract_parameters(&operation(operation_json)), expected);
}

#[test]
fn parameters_keep_their_document_order() {
    let extracted = extract_parameters(&operation(
        r#"{"parameters":[{"name":"z","in":"query"},{"name":"a","in":"query"}]}"#,
    ));
    assert_eq!(extracted.query, vec!["z".to_owned(), "a".to_owned()]);
}
