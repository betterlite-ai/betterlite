use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_pricing::unpriced::{
    model_has_no_cost_mapping, unpriced_models_block_message, unpriced_models_in_request,
};
use crate::auth_request_models::RequestedModels;

use super::support::{FakeCatalog, number, object, text};

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

fn many(models: &[&str]) -> RequestedModels {
    RequestedModels::Many(models.iter().map(|model| (*model).to_owned()).collect())
}

fn known(catalog: &mut FakeCatalog, model: &str) {
    catalog.group(model, None, None);
}

// a deployment with a resolved model_info entry, reached by its deployment id and provider model name
fn with_resolved(catalog: &mut FakeCatalog, model: &str, entry: JsonObject) {
    let deployment_id = format!("{model}-dep");
    catalog.deployed(
        model,
        object(&[("model", text("provider-model"))]),
        object(&[("id", text(&deployment_id))]),
    );
    catalog.resolved(&deployment_id, "provider-model", entry);
}

#[test]
fn override_key_declares_price() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "token-override");
    catalog.deployed(
        "token-override",
        object(&[("input_cost_per_token", number("0"))]),
        object(&[]),
    );
    known(&mut catalog, "null-override");
    catalog.deployed(
        "null-override",
        object(&[("custom_cost_per_second", Json::Null)]),
        object(&[]),
    );
    assert!(!model_has_no_cost_mapping(Some("token-override"), Some(&catalog)).unwrap());
    assert!(!model_has_no_cost_mapping(Some("null-override"), Some(&catalog)).unwrap());
}

#[test]
fn null_tiered_override_declares() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "tiered-override");
    catalog.deployed(
        "tiered-override",
        object(&[("tiered_pricing", Json::Null)]),
        object(&[]),
    );
    assert!(!model_has_no_cost_mapping(Some("tiered-override"), Some(&catalog)).unwrap());
}

#[test]
fn resolved_tiered_nonnull_priced() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "tiered");
    with_resolved(
        &mut catalog,
        "tiered",
        object(&[("tiered_pricing", Json::Null)]),
    );
    assert!(
        model_has_no_cost_mapping(Some("tiered"), Some(&catalog)).unwrap(),
        "a null resolved tiered_pricing does not price the group"
    );
    catalog.resolved(
        "tiered-dep",
        "provider-model",
        object(&[("tiered_pricing", Json::List(Vec::new()))]),
    );
    assert!(!model_has_no_cost_mapping(Some("tiered"), Some(&catalog)).unwrap());
}

#[test]
fn nested_positive_metric() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "nested");
    with_resolved(
        &mut catalog,
        "nested",
        object(&[(
            "cost_per_image",
            Json::Object(object(&[("low_quality", number("0.02"))])),
        )]),
    );
    assert!(!model_has_no_cost_mapping(Some("nested"), Some(&catalog)).unwrap());
}

#[test]
fn bool_string_not_price() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "not-numbers");
    with_resolved(
        &mut catalog,
        "not-numbers",
        object(&[
            ("cost_per_token", Json::Bool(true)),
            ("cost_per_second", text("0.5")),
            ("cost_per_page", number("0")),
        ]),
    );
    assert!(
        model_has_no_cost_mapping(Some("not-numbers"), Some(&catalog)).unwrap(),
        "a bool, a string and a zero are not positive numeric prices"
    );
}

#[test]
fn deep_nested_not_price() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "deep");
    with_resolved(
        &mut catalog,
        "deep",
        object(&[
            (
                "cost_per_region",
                Json::Object(object(&[(
                    "us",
                    Json::Object(object(&[("input", number("1"))])),
                )])),
            ),
            ("cost_per_list", Json::List(vec![number("1")])),
        ]),
    );
    assert!(
        model_has_no_cost_mapping(Some("deep"), Some(&catalog)).unwrap(),
        "only one nested level of a dict is read"
    );
}

#[test]
fn unknown_group_not_unpriced() {
    let catalog = FakeCatalog::default();
    assert!(!model_has_no_cost_mapping(Some("ghost"), Some(&catalog)).unwrap());
    assert!(!model_has_no_cost_mapping(Some(""), Some(&catalog)).unwrap());
    assert!(!model_has_no_cost_mapping(None, Some(&catalog)).unwrap());
    assert!(!model_has_no_cost_mapping(Some("ghost"), None).unwrap());
}

#[test]
fn duplicates_order_kept() {
    let mut catalog = FakeCatalog::default();
    known(&mut catalog, "b");
    known(&mut catalog, "a");
    known(&mut catalog, "p");
    catalog.deployed("p", object(&[]), object(&[("id", text("p-dep"))]));
    catalog.raw("p-dep", object(&[("output_cost_per_token", number("0"))]));
    assert_eq!(
        unpriced_models_in_request(&many(&["b", "a", "b"]), Some(&catalog)).unwrap(),
        vec!["b".to_owned(), "a".to_owned(), "b".to_owned()]
    );
    assert_eq!(
        unpriced_models_in_request(&many(&["b", "p", "a"]), Some(&catalog)).unwrap(),
        vec!["b".to_owned(), "a".to_owned()]
    );
    assert_eq!(
        unpriced_models_in_request(&one("a"), Some(&catalog)).unwrap(),
        vec!["a".to_owned()]
    );
    assert!(
        unpriced_models_in_request(&RequestedModels::None, Some(&catalog))
            .unwrap()
            .is_empty()
    );
    assert!(
        unpriced_models_in_request(&one("a"), None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn singular_plural_message() {
    let one_model = vec!["gpt-4o".to_owned()];
    assert_eq!(
        unpriced_models_block_message(&one_model),
        "Model 'gpt-4o' has no pricing in the cost map, so litellm cannot price the request. \
         Requests for unpriced models are blocked because \
         'block_requests_for_models_without_pricing' is enabled. Add pricing \
         (input_cost_per_token/output_cost_per_token) to allow the request."
    );
    let two_models = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(
        unpriced_models_block_message(&two_models),
        "Models 'a', 'b' have no pricing in the cost map, so litellm cannot price the request. \
         Requests for unpriced models are blocked because \
         'block_requests_for_models_without_pricing' is enabled. Add pricing \
         (input_cost_per_token/output_cost_per_token) to allow the request."
    );
}
