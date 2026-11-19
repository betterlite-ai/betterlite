use serde_json::{Map, Value};

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected an object, got {other}"),
    }
}

mod passthrough_extract_model {
    use rstest::rstest;
    use serde_json::json;

    use super::object;
    use crate::passthrough::vertex_setup::extract_vertex_setup_model;

    #[test]
    fn a_direct_model_path_yields_the_bare_name() {
        let setup = object(json!({"model": "projects/p/locations/l/models/gemini-live"}));
        assert_eq!(
            extract_vertex_setup_model(&setup).as_deref(),
            Some("gemini-live")
        );
    }

    #[test]
    fn a_nested_setup_model_yields_the_bare_name() {
        let setup =
            object(json!({"setup": {"model": "projects/p/locations/l/models/gemini-live"}}));
        assert_eq!(
            extract_vertex_setup_model(&setup).as_deref(),
            Some("gemini-live")
        );
    }

    #[test]
    fn a_direct_model_wins_over_the_nested_one() {
        let setup = object(json!({
            "model": "projects/p/models/direct",
            "setup": {"model": "projects/p/models/nested"}
        }));
        assert_eq!(
            extract_vertex_setup_model(&setup).as_deref(),
            Some("direct")
        );
    }

    #[test]
    fn a_direct_model_of_another_type_hides_the_nested_one() {
        let setup = object(json!({
            "model": 3,
            "setup": {"model": "projects/p/models/nested"}
        }));
        assert_eq!(extract_vertex_setup_model(&setup), None);
    }

    #[test]
    fn the_last_models_separator_wins() {
        let setup = object(json!({"model": "projects/p/models/a/models/b"}));
        assert_eq!(extract_vertex_setup_model(&setup).as_deref(), Some("b"));
    }

    #[test]
    fn a_path_without_the_separator_yields_nothing() {
        let setup = object(json!({"model": "gemini-live"}));
        assert_eq!(extract_vertex_setup_model(&setup), None);
    }

    #[test]
    fn a_path_that_ends_with_the_separator_yields_an_empty_name() {
        let setup = object(json!({"model": "projects/p/models/"}));
        assert_eq!(extract_vertex_setup_model(&setup).as_deref(), Some(""));
    }

    #[rstest]
    #[case::no_model(json!({"other": 1}))]
    #[case::setup_not_an_object(json!({"setup": "projects/p/models/m"}))]
    #[case::setup_without_a_model(json!({"setup": {"other": 1}}))]
    #[case::setup_model_of_another_type(json!({"setup": {"model": null}}))]
    fn nothing_is_extracted(#[case] setup: serde_json::Value) {
        assert_eq!(extract_vertex_setup_model(&object(setup)), None);
    }
}
