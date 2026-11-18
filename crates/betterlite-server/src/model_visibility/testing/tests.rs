#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::model_catalog_lookup::error::LookupError;

use super::{Row, Snapshot};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[test]
fn builds_the_declared_rows_aliases_and_provider() {
    let params = object(vec![("model", text("openai/gpt-4o"))]);
    let identified = Json::Object(object(vec![("id", text("a"))]));
    let missing = Json::Null;
    let declared = [
        Row {
            model_name: "visible",
            litellm_params: &params,
            model_info: &identified,
        },
        Row {
            model_name: "other",
            litellm_params: &params,
            model_info: &missing,
        },
    ];
    let aliases = object(vec![("alias", text("visible"))]);
    let snapshot = Snapshot::build(&declared, &aliases).unwrap();
    let names: Vec<&str> = snapshot
        .catalog
        .models
        .iter()
        .map(|row| row.model_name.as_str())
        .collect();
    assert_eq!(names, ["visible", "other"]);
    let lookup = snapshot.lookup();
    let resolved = lookup.get_model_list(Some("alias"), None).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].model_name, "alias");
    assert_eq!(resolved[0].model_info.get("id"), Some(&text("a")));
    assert_eq!(
        (lookup.inferred_provider)("gpt-4o"),
        Some("openai".to_owned())
    );
    assert_eq!(
        (lookup.inferred_provider)("wat/other"),
        Some("None".to_owned())
    );
}

#[test]
fn unbacked_pattern_fails_a_listing_that_matches_it() {
    let params = object(vec![("model", text("openai/gpt-4o"))]);
    let identified = Json::Object(object(vec![("id", text("a"))]));
    let declared = [Row {
        model_name: "visible",
        litellm_params: &params,
        model_info: &identified,
    }];
    let snapshot = Snapshot::build(&declared, &JsonObject::new())
        .unwrap()
        .with_unbacked_pattern()
        .unwrap();
    let lookup = snapshot.lookup();
    let backed = lookup.get_model_list(Some("visible"), None).unwrap();
    assert_eq!(backed.len(), 1);
    let failure = lookup.get_model_list(Some("broken-x"), None).unwrap_err();
    assert!(matches!(failure, LookupError::Route(_)));
}
