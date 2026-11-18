use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::semantic_guard::semantic_guard_route_loading::build_routes;
use crate::semantic_guard::semantic_guard_route_loading::custom_routes::{
    self, CustomRoutesError, MAX_ROUTES_FILE_BYTES,
};
use crate::semantic_guard::semantic_guard_route_loading::definition::DefinitionError;
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;

use super::support::{array, object, routes_file, sources, text};

#[test]
fn a_missing_file_is_reported_as_not_found() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("missing.yaml");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::FileNotFound { path: reported }) => assert_eq!(reported, path),
        other => panic!("expected a missing file error, got {other:?}"),
    }
}

#[test]
fn a_missing_file_message_names_the_path() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("missing.yaml");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap_err().to_string(),
        format!(
            "SemanticGuard: custom routes file not found: {}",
            path.display()
        )
    );
}

#[test]
fn an_invalid_yaml_file_is_reported() {
    let (_folder, path) = routes_file("[");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::Yaml { .. },
            ..
        })
    ));
}

#[test]
fn a_file_entry_missing_utterances_names_the_field() {
    let (_folder, path) = routes_file("- route_name: lonely\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::CustomRoutes {
            source:
                CustomRoutesError::Definition {
                    index: 0,
                    source: DefinitionError::MissingField { field },
                },
            ..
        }) => assert_eq!(field, "utterances"),
        other => panic!("expected a missing utterances error, got {other:?}"),
    }
}

#[test]
fn a_file_entry_missing_a_route_name_names_the_field() {
    let (_folder, path) = routes_file("- utterances: [\"hello\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::CustomRoutes {
            source:
                CustomRoutesError::Definition {
                    index: 0,
                    source: DefinitionError::MissingField { field },
                },
            ..
        }) => assert_eq!(field, "route_name"),
        other => panic!("expected a missing route name error, got {other:?}"),
    }
}

#[test]
fn an_unknown_file_field_is_rejected() {
    let (_folder, path) = routes_file("- route_name: x\n  utterances: [\"a\"]\n  utterance: b\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    let error = build_routes(&sources).unwrap_err();
    assert!(matches!(
        error,
        RouteLoadError::CustomRoutes {
            source: CustomRoutesError::Yaml { .. },
            ..
        }
    ));
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn a_repeated_key_in_one_route_is_rejected() {
    let (_folder, path) =
        routes_file("- route_name: x\n  utterances: [\"a\"]\n  utterances: [\"b\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::Yaml { .. },
            ..
        })
    ));
}

#[test]
fn a_file_entry_with_a_non_string_utterance_is_rejected() {
    let (_folder, path) = routes_file("- route_name: x\n  utterances: [\"a\", 1]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::Yaml { .. },
            ..
        })
    ));
}

#[test]
fn a_file_entry_that_is_not_a_mapping_is_rejected() {
    let (_folder, path) = routes_file("- 1\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::Yaml { .. },
            ..
        })
    ));
}

#[test]
fn a_scalar_document_is_an_invalid_format() {
    let (_folder, path) = routes_file("just text\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::InvalidFormat,
            ..
        })
    ));
}

#[test]
fn an_empty_file_is_an_invalid_format() {
    let (_folder, path) = routes_file("");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::InvalidFormat,
            ..
        })
    ));
}

#[test]
fn a_null_document_is_an_invalid_format() {
    let (_folder, path) = routes_file("null\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::CustomRoutes {
            source: CustomRoutesError::InvalidFormat,
            ..
        })
    ));
}

#[test]
fn an_inline_entry_missing_utterances_is_typed() {
    let inline = vec![object(&[("route_name", text("chat"))])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::MissingField { field },
        }) => assert_eq!(field, "utterances"),
        other => panic!("expected a missing utterances error, got {other:?}"),
    }
}

#[test]
fn an_inline_entry_missing_a_route_name_is_typed() {
    let inline = vec![object(&[("utterances", array(&["hello"]))])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::MissingField { field },
        }) => assert_eq!(field, "route_name"),
        other => panic!("expected a missing route name error, got {other:?}"),
    }
}

#[test]
fn an_inline_entry_that_is_not_a_mapping_is_rejected() {
    let sources = sources(&[], None, vec![text("chat")], 0.5);
    assert!(matches!(
        build_routes(&sources),
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::NotAMapping,
        })
    ));
}

#[test]
fn an_unknown_inline_field_is_rejected() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
        ("utterance", array(&["typo"])),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::UnknownField { field },
        }) => assert_eq!(field, "utterance"),
        other => panic!("expected an unknown field error, got {other:?}"),
    }
}

#[test]
fn a_non_string_inline_utterance_is_rejected() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        (
            "utterances",
            GuardrailValue::Array(vec![text("ok"), GuardrailValue::SignedInteger(1)]),
        ),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::FieldType { field, expected },
        }) => {
            assert_eq!(field, "utterances");
            assert_eq!(expected, "a list of strings");
        }
        other => panic!("expected a field type error, got {other:?}"),
    }
}

#[test]
fn a_non_text_inline_route_name_is_rejected() {
    let inline = vec![object(&[
        ("route_name", GuardrailValue::SignedInteger(5)),
        ("utterances", array(&["hello"])),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::FieldType { field, .. },
        }) => assert_eq!(field, "route_name"),
        other => panic!("expected a field type error, got {other:?}"),
    }
}

#[test]
fn a_non_list_inline_utterances_field_is_rejected() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", text("hello")),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::FieldType { field, .. },
        }) => assert_eq!(field, "utterances"),
        other => panic!("expected a field type error, got {other:?}"),
    }
}

#[test]
fn a_text_inline_threshold_is_rejected() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
        ("similarity_threshold", text("0.8")),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::InlineDefinition {
            index: 0,
            source: DefinitionError::FieldType { field, .. },
        }) => assert_eq!(field, "similarity_threshold"),
        other => panic!("expected a field type error, got {other:?}"),
    }
}

#[test]
fn a_non_finite_inline_threshold_is_rejected() {
    let inline = vec![object(&[
        ("route_name", text("nan")),
        ("utterances", array(&["hello"])),
        ("similarity_threshold", GuardrailValue::Decimal(f64::NAN)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    match build_routes(&sources) {
        Err(RouteLoadError::NonFiniteThreshold { name }) => assert_eq!(name, "nan"),
        other => panic!("expected a non-finite threshold error, got {other:?}"),
    }
}

#[test]
fn an_oversized_input_is_rejected() {
    let bytes = vec![b' '; MAX_ROUTES_FILE_BYTES + 1];
    assert!(matches!(
        custom_routes::decode(&bytes),
        Err(CustomRoutesError::TooLarge { limit }) if limit == MAX_ROUTES_FILE_BYTES
    ));
}

#[test]
fn an_input_at_the_byte_limit_is_read() {
    let prefix = "- route_name: big\n  utterances: [\"";
    let suffix = "\"]\n";
    let filler = "a".repeat(MAX_ROUTES_FILE_BYTES - prefix.len() - suffix.len());
    let text = format!("{prefix}{filler}{suffix}");
    assert_eq!(text.len(), MAX_ROUTES_FILE_BYTES);
    let routes = custom_routes::decode(text.as_bytes()).unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].route_name, "big");
}
