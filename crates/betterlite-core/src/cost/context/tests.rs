#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_logging::fields::timestamps::Moment;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::tiered_pricing::Scalar;
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::error::BillingContextError;
use super::ocr::ocr_info;
use super::pricing::{PricingQuery, deployment_info, pricing_entry};
use super::response_time::{ResponseTiming, response_time_ms};
use super::storage::store_cost_breakdown;
use super::{CostResponse, IdentityRequest, ProviderInference, billing_model_name};

struct Lookup {
    raw: BTreeMap<String, PriceRecord>,
    effective: BTreeMap<String, PriceRecord>,
}

impl Lookup {
    fn new(entries: &[(&str, &[(&str, Scalar)])]) -> Self {
        Self {
            raw: entries
                .iter()
                .map(|(key, fields)| ((*key).to_owned(), record(fields)))
                .collect(),
            effective: BTreeMap::new(),
        }
    }
}

impl PriceLookup for Lookup {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.effective.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.raw.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

#[derive(Clone, Debug)]
struct Inference {
    provider: Result<ProviderIdentity, &'static str>,
}

impl ProviderInference for Inference {
    type Failure = &'static str;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Self::Failure> {
        self.provider.clone()
    }
}

fn known(provider: ProviderId) -> Inference {
    Inference {
        provider: Ok(ProviderIdentity::Known(provider)),
    }
}

fn custom(slug: &'static str) -> Inference {
    Inference {
        provider: Ok(ProviderIdentity::Custom(slug.to_owned())),
    }
}

fn unavailable() -> Inference {
    Inference {
        provider: Err("the model has no provider"),
    }
}

fn number(text: &str) -> Scalar {
    Scalar::Number(serde_json::Number::from_str(text).expect("the fixture is a number"))
}

fn text(value: &str) -> Scalar {
    Scalar::Text(value.to_owned())
}

fn record(fields: &[(&str, Scalar)]) -> PriceRecord {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

// Scalar carries no PartialEq, so a record compares through its debug text instead.
fn rendered(record: &PriceRecord) -> BTreeMap<String, String> {
    record
        .iter()
        .map(|(key, value)| (key.clone(), format!("{value:?}")))
        .collect()
}

fn hidden(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn query<'query>(
    custom_pricing: Option<bool>,
    router_model_id: Option<&'query str>,
    logging_params: Option<&'query IndexMap<String, Json>>,
) -> PricingQuery<'query> {
    PricingQuery {
        custom_pricing,
        router_model_id,
        logging_params,
    }
}

fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn request<'request>(
    model: Option<&'request str>,
    response: Option<&'request CostResponse<'request>>,
    custom_llm_provider: Option<&'request str>,
) -> IdentityRequest<'request> {
    IdentityRequest {
        model,
        response,
        base_model: None,
        custom_pricing: None,
        custom_llm_provider,
        router_model_id: None,
        region_name: None,
    }
}

#[rstest]
#[case::explicit_provider(Some("openai"), known(ProviderId::Anthropic), "openai/gpt-4o")]
#[case::empty_explicit_provider(Some(""), unavailable(), "/gpt-4o")]
#[case::explicit_custom_provider(Some("my-provider"), unavailable(), "my-provider/gpt-4o")]
#[case::inferred_provider(None, known(ProviderId::Openai), "openai/gpt-4o")]
#[case::inferred_custom_provider(None, custom("my-provider"), "my-provider/gpt-4o")]
#[case::failed_inference(None, unavailable(), "gpt-4o")]
fn provider_selection_prefers_a_supplied_provider(
    #[case] provider: Option<&str>,
    #[case] inference: Inference,
    #[case] expected: &str,
) {
    let identity = request(Some("gpt-4o"), None, provider);
    let selected = billing_model_name(&identity, &Lookup::new(&[]), &inference);
    assert_eq!(selected.as_deref(), Some(expected));
}

#[test]
fn a_missing_model_and_provider_select_nothing() {
    let identity = request(None, None, None);
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()),
        None
    );
}

#[rstest]
#[case::zero_token_price(&[("input_cost_per_token", "0")], "openai/router")]
#[case::tiered_pricing_declared(&[("tiered_pricing", "0")], "openai/router")]
#[case::seconds_rate_declared(&[("cost_per_second", "0")], "openai/router")]
#[case::image_only(&[("input_cost_per_image", "0.04")], "openai/request-model")]
#[case::null_token_price(&[("input_cost_per_token", "null")], "openai/request-model")]
#[case::empty_entry(&[], "openai/request-model")]
fn custom_pricing_prefers_a_declared_router_entry(
    #[case] fields: &[(&str, &str)],
    #[case] expected: &str,
) {
    let entry: Vec<(&str, Scalar)> = fields
        .iter()
        .map(|(key, value)| {
            let scalar = if *value == "null" {
                Scalar::Null
            } else {
                number(value)
            };
            (*key, scalar)
        })
        .collect();
    let lookup = Lookup::new(&[("router", entry.as_slice())]);
    let mut identity = request(Some("request-model"), None, Some("openai"));
    identity.custom_pricing = Some(true);
    identity.router_model_id = Some("router");
    assert_eq!(
        billing_model_name(&identity, &lookup, &unavailable()).as_deref(),
        Some(expected)
    );
}

#[test]
fn an_absent_router_id_falls_back_to_the_request_model() {
    let mut identity = request(Some("request-model"), None, Some("openai"));
    identity.custom_pricing = Some(true);
    identity.router_model_id = Some("missing");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/request-model")
    );
}

#[test]
fn a_base_model_beats_the_provider_response_model() {
    let hidden_params = hidden(&[("provider_response_model", Json::Text("hidden".to_owned()))]);
    let response = CostResponse::Object {
        model: Some("response-model"),
        hidden_params: Some(&hidden_params),
        response_ms: None,
    };
    let mut identity = request(Some("request-model"), Some(&response), Some("openai"));
    identity.base_model = Some("base-model");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/base-model")
    );
    identity.base_model = Some("");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/")
    );
}

#[test]
fn the_provider_response_model_beats_the_response_model() {
    let hidden_params = hidden(&[("provider_response_model", Json::Text("hidden".to_owned()))]);
    let response = CostResponse::Object {
        model: Some("response-model"),
        hidden_params: Some(&hidden_params),
        response_ms: None,
    };
    let identity = request(Some("request-model"), Some(&response), Some("openai"));
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/hidden")
    );
}

#[test]
fn hidden_values_must_be_nonempty_strings() {
    let empty = hidden(&[
        ("provider_response_model", Json::Text(String::new())),
        ("model", Json::Text(String::new())),
    ]);
    let response = CostResponse::Object {
        model: None,
        hidden_params: Some(&empty),
        response_ms: None,
    };
    let identity = request(Some("request-model"), Some(&response), Some("openai"));
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/request-model")
    );

    let numeric = hidden(&[("model", Json::Number(serde_json::Number::from(5_u8)))]);
    let numeric_response = CostResponse::Object {
        model: None,
        hidden_params: Some(&numeric),
        response_ms: None,
    };
    let numeric_identity = request(
        Some("request-model"),
        Some(&numeric_response),
        Some("openai"),
    );
    assert_eq!(
        billing_model_name(&numeric_identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/request-model")
    );
}

#[test]
fn a_hidden_model_fills_a_missing_response_model() {
    let hidden_params = hidden(&[("model", Json::Text("hidden-model".to_owned()))]);
    let response = CostResponse::Object {
        model: None,
        hidden_params: Some(&hidden_params),
        response_ms: None,
    };
    let identity = request(Some("request-model"), Some(&response), Some("openai"));
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("openai/hidden-model")
    );
}

#[test]
fn a_dictionary_response_never_supplies_hidden_params() {
    let response = CostResponse::Dictionary {
        model: Some("dict-model"),
    };
    let identity = request(Some("request-model"), Some(&response), Some("p"));
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("p/dict-model")
    );
}

#[rstest]
#[case::known_head("openai/zzz", Some("azure"), "openai/zzz")]
#[case::registry_only_head("crusoe/zzz", Some("openai"), "openai/crusoe/zzz")]
#[case::unknown_head("zzz", Some("openai"), "openai/zzz")]
fn a_name_keeps_its_head_only_when_that_head_is_an_llm_provider(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: &str,
) {
    let identity = request(Some(model), None, provider);
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some(expected)
    );
}

#[test]
fn a_priced_region_joins_the_prefix() {
    let response = CostResponse::Dictionary {
        model: Some("response-model"),
    };
    let mut identity = request(Some("request-model"), Some(&response), Some("vertex_ai"));
    identity.region_name = Some("us-east5");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("vertex_ai/us-east5/response-model")
    );

    identity.region_name = Some("");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("vertex_ai//response-model")
    );
}

#[test]
fn a_hidden_region_beats_a_supplied_one() {
    let hidden_params = hidden(&[("region_name", Json::Text("r1".to_owned()))]);
    let response = CostResponse::Object {
        model: Some("response-model"),
        hidden_params: Some(&hidden_params),
        response_ms: None,
    };
    let mut identity = request(Some("request-model"), Some(&response), Some("p"));
    identity.region_name = Some("r2");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("p/r1/response-model")
    );
}

#[test]
fn explicit_pricing_and_missing_response_models_drop_the_region() {
    let response = CostResponse::Dictionary {
        model: Some("response-model"),
    };
    let mut identity = request(Some("request-model"), Some(&response), Some("p"));
    identity.region_name = Some("r1");
    identity.base_model = Some("base-model");
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("p/base-model")
    );

    let no_model = CostResponse::Dictionary { model: None };
    let mut unpriced = request(Some("request-model"), Some(&no_model), Some("p"));
    unpriced.region_name = Some("r1");
    assert_eq!(
        billing_model_name(&unpriced, &Lookup::new(&[]), &unavailable()).as_deref(),
        Some("p/request-model")
    );
}

#[rstest]
#[case::drops_an_alias_segment(&["openai/b/c"], "a/b/c", "openai/b/c")]
#[case::stops_at_a_provider_segment(&["openai/cohere"], "a/cohere", "openai/cohere")]
#[case::keeps_the_whole_name_when_the_tail_is_registered(&["a/b"], "a/b", "openai/a/b")]
#[case::leaves_an_unregistered_name(&[], "a/cohere", "openai/a/cohere")]
fn alias_stripping_tries_shorter_tail_prefixes(
    #[case] registered: &[&str],
    #[case] model: &str,
    #[case] expected: &str,
) {
    let fields: &[(&str, Scalar)] = &[("input_cost_per_token", number("1"))];
    let entries: Vec<(&str, &[(&str, Scalar)])> =
        registered.iter().map(|key| (*key, fields)).collect();
    let identity = request(Some(model), None, Some("openai"));
    assert_eq!(
        billing_model_name(&identity, &Lookup::new(&entries), &unavailable()).as_deref(),
        Some(expected)
    );
}

#[test]
fn alias_stripping_keeps_a_region_head() {
    let lookup = Lookup::new(&[("p/a/y", &[("input_cost_per_token", number("1"))])]);
    let response = CostResponse::Dictionary { model: Some("x/y") };
    let mut identity = request(Some("x/y"), Some(&response), Some("p"));
    identity.region_name = Some("a");
    assert_eq!(
        billing_model_name(&identity, &lookup, &unavailable()).as_deref(),
        Some("p/a/y")
    );
}

fn model_info(value: Json) -> Json {
    Json::Object(hidden(&[("model_info", value)]))
}

#[test]
fn deployment_info_needs_custom_pricing() {
    let params = params(&[("metadata", model_info(Json::Object(IndexMap::new())))]);
    assert!(
        deployment_info(&query(None, None, Some(&params)), &Lookup::new(&[]))
            .expect("the fixture is valid")
            .is_none()
    );
    assert!(
        deployment_info(&query(Some(false), None, Some(&params)), &Lookup::new(&[]))
            .expect("the fixture is valid")
            .is_none()
    );
}

#[test]
fn a_registered_router_entry_wins_even_when_empty() {
    let lookup = Lookup::new(&[("router", &[])]);
    let params = params(&[("metadata", model_info(Json::Object(IndexMap::new())))]);
    let deployment = deployment_info(&query(Some(true), Some("router"), Some(&params)), &lookup)
        .expect("the fixture is valid");
    assert_eq!(deployment.map(|record| record.len()), Some(0));
}

#[test]
fn deployment_info_reads_the_first_nonnull_model_info() {
    let logging_params = params(&[
        ("metadata", model_info(Json::Null)),
        (
            "litellm_metadata",
            model_info(Json::Object(hidden(&[(
                "input_cost_per_token",
                Json::Number(serde_json::Number::from(2_u8)),
            )]))),
        ),
    ]);
    let deployment = deployment_info(
        &query(Some(true), None, Some(&logging_params)),
        &Lookup::new(&[]),
    )
    .expect("the fixture is valid");
    assert_eq!(
        deployment.as_ref().map(rendered),
        Some(rendered(&record(&[("input_cost_per_token", number("2"))])))
    );
    let empty_metadata = params(&[("metadata", Json::Object(IndexMap::new()))]);
    assert!(
        deployment_info(
            &query(Some(true), None, Some(&empty_metadata)),
            &Lookup::new(&[])
        )
        .expect("the fixture is valid")
        .is_none()
    );
}

#[test]
fn deployment_info_rejects_a_nonobject_metadata() {
    let text_metadata = params(&[("metadata", Json::Text("metadata".to_owned()))]);
    assert!(matches!(
        deployment_info(
            &query(Some(true), None, Some(&text_metadata)),
            &Lookup::new(&[])
        ),
        Err(BillingContextError::MetadataShape { field: "metadata" })
    ));
    let wrong_model_info = params(&[("metadata", model_info(Json::Bool(true)))]);
    assert!(matches!(
        deployment_info(
            &query(Some(true), None, Some(&wrong_model_info)),
            &Lookup::new(&[])
        ),
        Err(BillingContextError::ModelInfoShape)
    ));
    let listed = params(&[("metadata", Json::List(vec![Json::Bool(true)]))]);
    assert!(matches!(
        deployment_info(&query(Some(true), None, Some(&listed)), &Lookup::new(&[])),
        Err(BillingContextError::MetadataShape { field: "metadata" })
    ));
    let empty_list = params(&[("metadata", Json::List(Vec::new()))]);
    assert!(
        deployment_info(
            &query(Some(true), None, Some(&empty_list)),
            &Lookup::new(&[])
        )
        .expect("the fixture is valid")
        .is_none()
    );
}

#[test]
fn a_numeric_or_empty_metadata_is_rejected_only_when_truthy() {
    let counted = params(&[("metadata", Json::Number(serde_json::Number::from(2_u8)))]);
    assert!(matches!(
        deployment_info(&query(Some(true), None, Some(&counted)), &Lookup::new(&[])),
        Err(BillingContextError::MetadataShape { field: "metadata" })
    ));
    let zero = params(&[("metadata", Json::Number(serde_json::Number::from(0_u8)))]);
    assert!(
        deployment_info(&query(Some(true), None, Some(&zero)), &Lookup::new(&[]))
            .expect("a zero metadata is skipped")
            .is_none()
    );
    let empty_text = params(&[("metadata", Json::Text(String::new()))]);
    assert!(
        deployment_info(
            &query(Some(true), None, Some(&empty_text)),
            &Lookup::new(&[])
        )
        .expect("an empty metadata is skipped")
        .is_none()
    );
}

#[test]
fn ocr_info_layers_logging_params_over_the_deployment_record() {
    let deployment = model_info(Json::Object(hidden(&[
        (
            "ocr_cost_per_page",
            Json::Number(serde_json::Number::from(2_u8)),
        ),
        (
            "annotation_cost_per_page",
            Json::Number(serde_json::Number::from(3_u8)),
        ),
    ])));
    let params = params(&[
        ("metadata", deployment),
        (
            "ocr_cost_per_page",
            Json::Number(serde_json::Number::from(1_u8)),
        ),
    ]);
    let pricing = ocr_info(&query(Some(true), None, Some(&params)), &Lookup::new(&[]))
        .expect("the fixture is valid")
        .expect("the deployment record prices OCR");
    assert_eq!(pricing.ocr_cost_per_page, Some(Decimal::ONE));
    assert_eq!(pricing.annotation_cost_per_page, Some(Decimal::from(3_u8)));
    assert_eq!(pricing.ocr_cost_per_credit, None);
}

#[test]
fn ocr_info_reads_the_deployment_record_without_logging_params() {
    let lookup = Lookup::new(&[("router", &[("ocr_cost_per_page", number("2"))])]);
    let pricing = ocr_info(&query(Some(true), Some("router"), None), &lookup)
        .expect("the fixture is valid")
        .expect("the deployment record prices OCR");
    assert_eq!(pricing.ocr_cost_per_page, Some(Decimal::from(2_u8)));
    assert_eq!(pricing.ocr_cost_per_credit, None);
}

#[test]
fn ocr_info_without_a_custom_price_has_nothing_to_layer() {
    let params = params(&[(
        "ocr_cost_per_page",
        Json::Number(serde_json::Number::from(1_u8)),
    )]);
    assert!(
        ocr_info(&query(None, None, Some(&params)), &Lookup::new(&[]))
            .expect("the fixture is valid")
            .is_none()
    );
}

#[test]
fn pricing_entry_returns_the_deployment_record_first() {
    let lookup = Lookup::new(&[("router", &[("input_cost_per_token", number("1"))])]);
    let params = params(&[(
        "metadata",
        model_info(Json::Object(hidden(&[("a", Json::Bool(true))]))),
    )]);
    let mut identity = request(Some("request-model"), None, Some("openai"));
    identity.custom_pricing = Some(true);
    identity.router_model_id = Some("router");
    let entry = pricing_entry(&identity, Some(&params), &lookup, &unavailable())
        .expect("the fixture is valid")
        .expect("the deployment entry wins");
    assert_eq!(entry.0, "router");
    assert_eq!(
        rendered(&entry.1),
        rendered(&record(&[("input_cost_per_token", number("1"))]))
    );

    identity.router_model_id = None;
    let request_entry = pricing_entry(&identity, Some(&params), &lookup, &unavailable())
        .expect("the fixture is valid")
        .expect("the request model keys the deployment entry");
    assert_eq!(request_entry.0, "request-model");
    assert_eq!(
        rendered(&request_entry.1),
        rendered(&record(&[("a", Scalar::Bool(true))]))
    );
}

#[rstest]
#[case::selected_model("openai/response-model")]
#[case::response_model("response-model")]
#[case::request_model("request-model")]
fn pricing_entry_walks_the_candidate_models(#[case] expected_key: &str) {
    let mut effective = BTreeMap::new();
    effective.insert(
        expected_key.to_owned(),
        record(&[
            ("key", text(expected_key)),
            ("input_cost_per_token", number("2")),
        ]),
    );
    let lookup = Lookup {
        raw: BTreeMap::new(),
        effective,
    };
    let response = CostResponse::Dictionary {
        model: Some("response-model"),
    };
    let identity = request(Some("request-model"), Some(&response), Some("openai"));
    let entry = pricing_entry(&identity, None, &lookup, &unavailable())
        .expect("the fixture is valid")
        .expect("a candidate resolves");
    assert_eq!(entry.0, expected_key);
    assert_eq!(
        rendered(&entry.1),
        rendered(&record(&[
            ("key", text(expected_key)),
            ("input_cost_per_token", number("2"))
        ]))
    );
}

#[test]
fn pricing_entry_skips_an_empty_candidate_name() {
    let mut effective = BTreeMap::new();
    effective.insert(
        String::new(),
        record(&[("key", text("")), ("input_cost_per_token", number("1"))]),
    );
    effective.insert(
        "request-model".to_owned(),
        record(&[
            ("key", text("request-model")),
            ("input_cost_per_token", number("2")),
        ]),
    );
    let lookup = Lookup {
        raw: BTreeMap::new(),
        effective,
    };
    let response = CostResponse::Dictionary { model: Some("") };
    let identity = request(Some("request-model"), Some(&response), Some("openai"));
    let entry = pricing_entry(&identity, None, &lookup, &unavailable())
        .expect("the fixture is valid")
        .expect("the request model resolves");
    assert_eq!(entry.0, "request-model");
}

#[test]
fn pricing_entry_prefers_a_nonempty_raw_entry_over_the_effective_one() {
    let mut effective = BTreeMap::new();
    effective.insert(
        "request-model".to_owned(),
        record(&[
            ("key", text("request-model")),
            ("input_cost_per_token", number("2")),
        ]),
    );
    let lookup = Lookup {
        raw: BTreeMap::from([(
            "request-model".to_owned(),
            record(&[("input_cost_per_token", number("1"))]),
        )]),
        effective,
    };
    let identity = request(Some("request-model"), None, None);
    let entry = pricing_entry(&identity, None, &lookup, &unavailable())
        .expect("the fixture is valid")
        .expect("a candidate resolves");
    assert_eq!(entry.0, "request-model");
    assert_eq!(
        rendered(&entry.1),
        rendered(&record(&[("input_cost_per_token", number("1"))]))
    );
}

#[test]
fn pricing_entry_falls_back_to_the_effective_record_for_an_empty_raw_one() {
    let mut effective = BTreeMap::new();
    effective.insert(
        "request-model".to_owned(),
        record(&[
            ("key", text("request-model")),
            ("input_cost_per_token", number("2")),
        ]),
    );
    let lookup = Lookup {
        raw: BTreeMap::from([("request-model".to_owned(), PriceRecord::new())]),
        effective,
    };
    let identity = request(Some("request-model"), None, None);
    let entry = pricing_entry(&identity, None, &lookup, &unavailable())
        .expect("the fixture is valid")
        .expect("a candidate resolves");
    assert_eq!(
        rendered(&entry.1),
        rendered(&record(&[
            ("key", text("request-model")),
            ("input_cost_per_token", number("2"))
        ]))
    );
}

#[test]
fn pricing_entry_errors_when_the_effective_record_has_no_key() {
    let mut effective = BTreeMap::new();
    effective.insert(
        "request-model".to_owned(),
        record(&[("input_cost_per_token", number("2"))]),
    );
    let lookup = Lookup {
        raw: BTreeMap::new(),
        effective,
    };
    let identity = request(Some("request-model"), None, None);
    assert!(matches!(
        pricing_entry(&identity, None, &lookup, &unavailable()),
        Err(BillingContextError::MissingKey { model }) if model == "request-model"
    ));
}

#[test]
fn pricing_entry_without_a_candidate_returns_nothing() {
    let identity = request(Some("request-model"), None, None);
    assert!(
        pricing_entry(&identity, None, &Lookup::new(&[]), &unavailable())
            .expect("the fixture is valid")
            .is_none()
    );
}

fn moment(value: &str) -> Moment {
    Moment::DateTime(DateTime::parse_from_rfc3339(value).expect("the fixture is RFC 3339"))
}

fn timing<'timing>(
    start: &'timing Moment,
    end: &'timing Moment,
    now: &'timing Moment,
) -> ResponseTiming<'timing> {
    ResponseTiming {
        start_time: Some(start),
        end_time: Some(end),
        now,
    }
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture is a decimal")
}

#[rstest]
#[case::stamped_zero("0", "0")]
#[case::stamped_negative("-1.5", "-1.5")]
#[case::stamped_fraction("0.25", "0.25")]
fn a_numeric_stamped_response_time_wins(#[case] stamped: &str, #[case] expected: &str) {
    let stamped_value = Json::Number(serde_json::Number::from_str(stamped).expect("a number"));
    let response = CostResponse::Object {
        model: None,
        hidden_params: None,
        response_ms: Some(&stamped_value),
    };
    let start = moment("2026-01-01T00:00:00+00:00");
    let end = moment("2026-01-01T00:00:01.500+00:00");
    let now = moment("2026-01-01T00:00:09+00:00");
    let observed = response_time_ms(
        Some(&response),
        Some(decimal("2.5")),
        Some(&timing(&start, &end, &now)),
    )
    .expect("the stamped time is a decimal");
    assert_eq!(observed, decimal(expected));
}

#[test]
fn a_boolean_stamped_response_time_counts_as_one_millisecond() {
    let stamped = Json::Bool(true);
    let response = CostResponse::Object {
        model: None,
        hidden_params: None,
        response_ms: Some(&stamped),
    };
    assert_eq!(
        response_time_ms(Some(&response), Some(decimal("2.5")), None)
            .expect("the stamped time is a decimal"),
        Decimal::ONE
    );
}

#[test]
fn a_non_numeric_stamped_value_is_ignored() {
    let stamped = Json::Text("12".to_owned());
    let response = CostResponse::Object {
        model: None,
        hidden_params: None,
        response_ms: Some(&stamped),
    };
    assert_eq!(
        response_time_ms(Some(&response), Some(decimal("5")), None)
            .expect("the total time is a decimal"),
        decimal("5")
    );
}

#[test]
fn a_dictionary_response_has_no_stamped_time() {
    let response = CostResponse::Dictionary { model: None };
    assert_eq!(
        response_time_ms(Some(&response), Some(decimal("5")), None)
            .expect("the total time is a decimal"),
        decimal("5")
    );
}

#[test]
fn a_nonzero_total_time_beats_the_logging_clock() {
    let start = moment("2026-01-01T00:00:00+00:00");
    let end = moment("2026-01-01T00:00:01.500+00:00");
    let now = moment("2026-01-01T00:00:09+00:00");
    let response = CostResponse::Dictionary { model: None };
    assert_eq!(
        response_time_ms(
            Some(&response),
            Some(decimal("-2.5")),
            Some(&timing(&start, &end, &now))
        )
        .expect("the total time is a decimal"),
        decimal("-2.5")
    );
    assert_eq!(
        response_time_ms(
            Some(&response),
            Some(Decimal::ZERO),
            Some(&timing(&start, &end, &now))
        )
        .expect("the logging clock is readable"),
        decimal("1500")
    );
    assert_eq!(
        response_time_ms(Some(&response), None, None).expect("zero needs no clock"),
        Decimal::ZERO
    );
}

struct FailingTarget;

#[derive(Debug)]
struct StorageFailure;

impl CostBreakdownTarget for FailingTarget {
    type Failure = StorageFailure;

    fn set_cost_breakdown(&mut self, _input: &CostBreakdownInput) -> Result<(), StorageFailure> {
        Err(StorageFailure)
    }
}

struct RecordingTarget {
    total: Option<Decimal>,
}

impl CostBreakdownTarget for RecordingTarget {
    type Failure = StorageFailure;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), StorageFailure> {
        self.total = Some(input.total_cost);
        Ok(())
    }
}

fn breakdown_input() -> CostBreakdownInput {
    CostBreakdownInput {
        input_cost: Decimal::ONE,
        output_cost: Decimal::ONE,
        total_cost: Decimal::from(3_u8),
        tool_usage_cost: Decimal::ZERO,
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        cache_read_cost: None,
        cache_creation_cost: None,
        reasoning_cost: None,
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        billed_token_rates: None,
    }
}

#[test]
fn a_failing_breakdown_target_does_not_fail_the_charge() {
    let mut target = FailingTarget;
    store_cost_breakdown(Some(&mut target), &breakdown_input());
}

#[test]
fn a_missing_breakdown_target_is_skipped() {
    store_cost_breakdown(None::<&mut FailingTarget>, &breakdown_input());
}

#[test]
fn a_working_breakdown_target_receives_the_amounts() {
    let mut target = RecordingTarget { total: None };
    store_cost_breakdown(Some(&mut target), &breakdown_input());
    assert_eq!(target.total, Some(Decimal::from(3_u8)));
}
