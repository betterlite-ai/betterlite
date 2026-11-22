#![cfg(test)]

use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::DeploymentBlobs;
use super::catalog::{cost_map_entry, loaded_catalog_entry};
use super::delta::{PtuPatch, PtuPricingDelta, ptu_pricing_delta};
use super::error::PtuPricingError;
use super::prices::{is_nonzero_price, is_nonzero_rate, is_zero_price};
use super::testing::CostMap;
use super::zeroed::{
    check_ptu_unpriced, ptu_priced_deployment, ptu_zeroed_pricing, write_ptu_pricing,
};
use crate::jobs::ptu::attribution::PtuAttribution;
use crate::model_effective_fields::error::Refusal;

const PTU_INFO: &str =
    r#"{"id": "m1", "team_id": "t1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5}"#;
const ZEROED: &str = r#"{
    "input_cost_per_token": 0.0, "output_cost_per_token": 0.0,
    "input_cost_per_character": 0.0, "output_cost_per_character": 0.0,
    "cache_read_input_token_cost": 0.0, "cache_creation_input_token_cost": 0.0,
    "cache_creation_input_token_cost_above_1hr": 0.0,
    "cache_creation_input_token_cost_above_200k_tokens": 0.0,
    "cache_read_input_token_cost_above_200k_tokens": 0.0,
    "google_maps_grounding_cost_per_query": 0.0,
    "tiered_pricing": [],
    "search_context_cost_per_query": {"search_context_size_low": 0.0, "search_context_size_medium": 0.0, "search_context_size_high": 0.0}
}"#;

struct Flag(bool);

impl PtuAttribution for Flag {
    fn enabled(&self) -> bool {
        self.0
    }
}

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn fake_cost_map(entries: &[(&str, &str)], broken: bool) -> CostMap {
    let entries = entries
        .iter()
        .map(|(model, entry)| ((*model).to_owned(), object(entry)))
        .collect();
    CostMap::new(entries, broken)
}

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b833-salt".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn written(outcome: Result<PtuPricingDelta, PtuPricingError>) -> JsonObject {
    let mut blob = JsonObject::new();
    write_ptu_pricing(&outcome.unwrap().written, &mut blob).unwrap();
    blob
}

#[rstest]
#[case::integer_zero("0", [false, false, true])]
#[case::float_zero("0.0", [false, false, true])]
#[case::negative_zero("-0.0", [false, false, true])]
#[case::exponent_zero("0e3", [false, false, true])]
#[case::zero_below_decimal_scale("0e-30", [false, false, true])]
#[case::positive_rate("0.000002", [true, true, false])]
#[case::negative_rate("-1", [true, true, false])]
#[case::rate_past_decimal_range("1e400", [true, true, false])]
#[case::rate_below_decimal_scale("1e-30", [true, true, false])]
#[case::true_flag("true", [false, false, false])]
#[case::false_flag("false", [false, false, false])]
#[case::text_rate(r#""0.5""#, [false, false, false])]
#[case::null("null", [false, false, false])]
#[case::empty_table("{}", [false, false, false])]
#[case::free_table(r#"{"search_context_size_low": 0, "search_context_size_high": 0.0}"#, [false, false, true])]
#[case::priced_table(r#"{"search_context_size_low": 0, "search_context_size_high": 0.01}"#, [false, true, false])]
#[case::nested_table(r#"{"tier": {"rate": 1}}"#, [false, false, true])]
#[case::empty_tiers("[]", [false, false, true])]
#[case::tiers(r#"[{"range": [0, 1000]}]"#, [false, false, false])]
fn explicit_zero_price(#[case] text: &str, #[case] expected: [bool; 3]) {
    let value: Json = serde_json::from_str(text).unwrap();
    let found = [
        is_nonzero_rate(&value),
        is_nonzero_price(&value),
        is_zero_price(&value),
    ];
    assert_eq!(found, expected);
}

#[rstest]
#[case::fixed_and_token_prices_together(
    PTU_INFO,
    r#"{"output_cost_per_token": 0.00001, "input_cost_per_token": 0.000002, "input_cost_per_second": 0}"#,
    true,
    Some(&["input_cost_per_token", "output_cost_per_token"][..])
)]
#[case::tier_table(PTU_INFO, r#"{"tiered_pricing": [{"range": [0, 1000]}]}"#, true, Some(&["tiered_pricing"][..]))]
#[case::empty_tiers(PTU_INFO, r#"{"tiered_pricing": []}"#, true, None)]
#[case::null_tiers(PTU_INFO, r#"{"tiered_pricing": null}"#, true, None)]
#[case::priced_search_table(
    PTU_INFO,
    r#"{"search_context_cost_per_query": {"search_context_size_low": 0.01}}"#,
    true,
    Some(&["search_context_cost_per_query"][..])
)]
#[case::zeroed_supply(
    PTU_INFO,
    r#"{"input_cost_per_token": 0, "search_context_cost_per_query": {"search_context_size_low": 0}}"#,
    true,
    None
)]
#[case::zero_below_decimal_scale(PTU_INFO, r#"{"input_cost_per_token": 0e-30}"#, true, None)]
#[case::non_rate_fields(PTU_INFO, r#"{"output_vector_size": 3, "rpm": 10}"#, true, None)]
#[case::attribution_off(PTU_INFO, r#"{"input_cost_per_token": 0.000002}"#, false, None)]
#[case::count_alone(
    r#"{"ptu_count": 2}"#,
    r#"{"input_cost_per_token": 0.000002}"#,
    true,
    None
)]
#[case::rate_alone(
    r#"{"cost_per_ptu_per_hour": 1.5}"#,
    r#"{"input_cost_per_token": 0.000002}"#,
    true,
    None
)]
#[case::rate_cleared(
    r#"{"ptu_count": 2, "cost_per_ptu_per_hour": null}"#,
    r#"{"input_cost_per_token": 0.000002}"#,
    true,
    None
)]
fn ptu_priced_conflict(
    #[case] model_info: &str,
    #[case] supplied: &str,
    #[case] enabled: bool,
    #[case] expected: Option<&[&str]>,
) {
    let outcome = check_ptu_unpriced(&object(model_info), &object(supplied), &Flag(enabled));
    let priced = match outcome {
        Ok(()) => None,
        Err(PtuPricingError::Priced { priced }) => Some(priced),
        Err(error @ PtuPricingError::Rate { .. }) => panic!("{error}"),
    };
    assert_eq!(priced.as_deref(), expected);
}

#[test]
fn priced_conflict_is_a_bare_400_before_any_zeroing() {
    let model_info = object(PTU_INFO);
    let params = object(r#"{"model": "azure/gpt-4o"}"#);
    let blobs = DeploymentBlobs {
        model_info: &model_info,
        litellm_params: &params,
    };
    let supplied = object(r#"{"input_cost_per_token": 0.000002, "tiered_pricing": [{}]}"#);
    let error = ptu_zeroed_pricing(blobs, &supplied, &Flag(true)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "A PTU deployment bills by reserved capacity, so input_cost_per_token, tiered_pricing \
         cannot be charged on top of it. Send 0 or no value, or remove ptu_count and \
         cost_per_ptu_per_hour to bill per token."
    );
    let refusal = Refusal::from(&error);
    assert_eq!(
        refusal,
        Refusal {
            status: 400,
            error_type: None,
            param: None
        }
    );
    let patch = PtuPatch {
        litellm_params: Some(&supplied),
        model_info: None,
    };
    let refused = ptu_pricing_delta(&model_info, blobs, patch, &Flag(true));
    assert!(matches!(refused, Err(PtuPricingError::Priced { .. })));
}

#[test]
fn price_delta() {
    let model_info = object(
        r#"{"id": "m1", "team_id": "t1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5, "input_cost_per_token": 0.000002}"#,
    );
    let params = object(
        r#"{"model": "azure/gpt-4o", "output_cost_per_second": 0.5, "input_cost_per_pixel": 0, "search_context_cost_per_query": {"search_context_size_low": 0.01}}"#,
    );
    let blobs = DeploymentBlobs {
        model_info: &model_info,
        litellm_params: &params,
    };
    let patch_params = object(r#"{"rpm": 5, "input_cost_per_token": null}"#);
    let patch = PtuPatch {
        litellm_params: Some(&patch_params),
        model_info: Some(&model_info),
    };
    let outcome = ptu_pricing_delta(&object("{}"), blobs, patch, &Flag(true));
    assert!(outcome.as_ref().unwrap().released.is_empty());
    let mut expected = object(ZEROED);
    expected.insert(
        "output_cost_per_second".to_owned(),
        Json::Number(serde_json::from_str("0.0").unwrap()),
    );
    assert_eq!(written(outcome), expected);
}

#[test]
fn price_delta_releases_nothing_while_ptu_stays() {
    let model_info = object(
        r#"{"id": "m1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5, "input_cost_per_token": 0.0}"#,
    );
    let params = object(r#"{"model": "azure/gpt-4o", "input_cost_per_token": 0.0}"#);
    let blobs = DeploymentBlobs {
        model_info: &model_info,
        litellm_params: &params,
    };
    let clears = object(r#"{"ptu_count": null}"#);
    let patch = PtuPatch {
        litellm_params: None,
        model_info: Some(&clears),
    };
    let outcome = ptu_pricing_delta(&object(PTU_INFO), blobs, patch, &Flag(true));
    assert!(outcome.as_ref().unwrap().released.is_empty());
    assert_eq!(written(outcome), object(ZEROED));
}

#[rstest]
#[case::attribution_off(PTU_INFO, false)]
#[case::not_ptu(r#"{"id": "m1", "input_cost_per_token": 0.000002}"#, true)]
fn price_delta_without_ptu_writes_nothing(#[case] model_info: &str, #[case] enabled: bool) {
    let info = object(model_info);
    let params = object(r#"{"model": "azure/gpt-4o", "input_cost_per_token": 0}"#);
    let blobs = DeploymentBlobs {
        model_info: &info,
        litellm_params: &params,
    };
    let patch = PtuPatch {
        litellm_params: None,
        model_info: None,
    };
    let delta = ptu_pricing_delta(&object(PTU_INFO), blobs, patch, &Flag(enabled)).unwrap();
    assert!(delta.written.entries.is_empty());
    assert!(delta.released.is_empty());
}

const RELEASED: [&str; 5] = [
    "cache_read_input_token_cost",
    "input_cost_per_second",
    "input_cost_per_token",
    "search_context_cost_per_query",
    "tiered_pricing",
];

#[rstest]
#[case::count_cleared(PTU_INFO, Some(r#"{"ptu_count": null}"#), true, &RELEASED[..])]
#[case::rate_cleared(PTU_INFO, Some(r#"{"cost_per_ptu_per_hour": null}"#), true, &RELEASED[..])]
#[case::stored_rate_alone(r#"{"cost_per_ptu_per_hour": 1.5}"#, Some(r#"{"cost_per_ptu_per_hour": null}"#), true, &RELEASED[..])]
#[case::only_window_cleared(PTU_INFO, Some(r#"{"ptu_effective_to": null}"#), true, &[][..])]
#[case::count_changed(PTU_INFO, Some(r#"{"ptu_count": 3}"#), true, &[][..])]
#[case::attribution_off(PTU_INFO, Some(r#"{"ptu_count": null}"#), false, &[][..])]
#[case::never_ptu(r#"{"id": "m1"}"#, Some(r#"{"ptu_count": null}"#), true, &[][..])]
#[case::stored_pair_null(r#"{"ptu_count": null, "cost_per_ptu_per_hour": null}"#, Some(r#"{"ptu_count": null}"#), true, &[][..])]
#[case::no_model_info_patch(PTU_INFO, None, true, &[][..])]
fn clear_ptu_field(
    #[case] stored: &str,
    #[case] patch_model_info: Option<&str>,
    #[case] enabled: bool,
    #[case] expected: &[&str],
) {
    // 0e-30 is a zero LiteLLM releases though its scale is past what Decimal can hold
    let model_info = object(
        r#"{"id": "m1", "input_cost_per_token": 0e-30, "output_cost_per_token": 0.00001, "tiered_pricing": [],
            "search_context_cost_per_query": {"search_context_size_low": 0.0, "search_context_size_high": 0}}"#,
    );
    let params = object(
        r#"{"model": "azure/gpt-4o", "cache_read_input_token_cost": 0, "input_cost_per_second": 0.0,
            "output_cost_per_token": 0.00001, "input_cost_per_character": {}, "rpm": 0}"#,
    );
    let blobs = DeploymentBlobs {
        model_info: &model_info,
        litellm_params: &params,
    };
    let clears = patch_model_info.map(object);
    let patch = PtuPatch {
        litellm_params: None,
        model_info: clears.as_ref(),
    };
    let delta = ptu_pricing_delta(&object(stored), blobs, patch, &Flag(enabled)).unwrap();
    assert!(delta.written.entries.is_empty());
    assert_eq!(delta.released.into_iter().collect::<Vec<_>>(), expected);
}

#[test]
fn priced_deployment_zeroes_both_blobs() {
    let mut params = object(r#"{"model": "azure/gpt-4o", "input_cost_per_token": 0}"#);
    let mut model_info = object(
        r#"{"id": "m1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5, "input_cost_per_token": 0, "ocr_cost_per_page": 0.01}"#,
    );
    ptu_priced_deployment(&mut params, &mut model_info, &Flag(true)).unwrap();
    let mut zeroed = object(ZEROED);
    zeroed.insert(
        "ocr_cost_per_page".to_owned(),
        Json::Number(serde_json::from_str("0.0").unwrap()),
    );
    let mut expected_params = object(r#"{"model": "azure/gpt-4o"}"#);
    expected_params.extend(zeroed.clone());
    let mut expected_info = object(r#"{"id": "m1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5}"#);
    expected_info.extend(zeroed);
    assert_eq!(params, expected_params);
    assert_eq!(model_info, expected_info);
}

#[rstest]
#[case::own_price(PTU_INFO, Flag(true), true)]
#[case::attribution_off(PTU_INFO, Flag(false), false)]
#[case::not_ptu(r#"{"id": "m1"}"#, Flag(true), false)]
fn priced_deployment_refuses_its_own_price(
    #[case] model_info: &str,
    #[case] attribution: Flag,
    #[case] refused: bool,
) {
    let mut params = object(r#"{"model": "azure/gpt-4o", "input_cost_per_token": 0.000002}"#);
    let mut info = object(model_info);
    let outcome = ptu_priced_deployment(&mut params, &mut info, &attribution);
    assert_eq!(outcome.is_err(), refused);
    assert_eq!(
        params,
        object(r#"{"model": "azure/gpt-4o", "input_cost_per_token": 0.000002}"#)
    );
    assert_eq!(info, object(model_info));
}

#[rstest]
#[case::base_model_wins(
    r#"{"base_model": "azure/gpt-4o-base"}"#,
    r#""azure/my-deployment""#,
    Some("azure/gpt-4o-base")
)]
#[case::stored_model(
    r#"{"id": "m1"}"#,
    r#""azure/my-deployment""#,
    Some("azure/my-deployment")
)]
#[case::number_base_model(
    r#"{"base_model": 7}"#,
    r#""azure/my-deployment""#,
    Some("azure/my-deployment")
)]
#[case::null_base_model(
    r#"{"base_model": null}"#,
    r#""azure/my-deployment""#,
    Some("azure/my-deployment")
)]
#[case::unmapped_model(
    r#"{"base_model": "azure/unmapped"}"#,
    r#""azure/my-deployment""#,
    Some("azure/unmapped")
)]
#[case::model_not_text(r#"{"id": "m1"}"#, "5", None)]
#[case::model_null(r#"{"id": "m1"}"#, "null", None)]
#[tokio::test]
async fn catalog_price(
    #[case] incoming: &str,
    #[case] stored_model: &str,
    #[case] expected_lookup: Option<&str>,
) {
    let cost_map = fake_cost_map(
        &[
            (
                "azure/gpt-4o-base",
                r#"{"key": "azure/gpt-4o-base", "input_cost_per_token": 0.0000025}"#,
            ),
            (
                "azure/my-deployment",
                r#"{"input_cost_per_token": 0.000001}"#,
            ),
        ],
        false,
    );
    let mut params = JsonObject::new();
    params.insert(
        "model".to_owned(),
        serde_json::from_str(stored_model).unwrap(),
    );
    let mut keys = ModelCostKeys::default();
    let entry = cost_map_entry(&cost_map, &mut keys, &cipher(), &params, &object(incoming)).await;
    let expected_entry = expected_lookup
        .and_then(|model| cost_map.entries.get(model).cloned())
        .unwrap_or_default();
    assert_eq!(entry, expected_entry);
    let lookups: Vec<String> = expected_lookup.into_iter().map(str::to_owned).collect();
    assert_eq!(cost_map.lookups(), lookups);
}

#[tokio::test]
async fn catalog_price_reads_the_decrypted_stored_model() {
    let sealed = StoredValue::<()>::Text(SecretString::from("azure/my-deployment".to_owned()));
    let StoredValue::Text(encrypted) = cipher().encrypt_value(sealed, None).unwrap() else {
        panic!("text seals to text");
    };
    let mut params = JsonObject::new();
    params.insert(
        "model".to_owned(),
        Json::Text(encrypted.expose_secret().to_owned()),
    );
    let cost_map = fake_cost_map(
        &[(
            "azure/my-deployment",
            r#"{"input_cost_per_token": 0.000001}"#,
        )],
        false,
    );
    let mut keys = ModelCostKeys::default();
    let entry = cost_map_entry(&cost_map, &mut keys, &cipher(), &params, &object("{}")).await;
    assert_eq!(entry, object(r#"{"input_cost_per_token": 0.000001}"#));
}

#[tokio::test]
async fn catalog_price_survives_an_unreachable_cost_map() {
    let cost_map = fake_cost_map(&[], true);
    let params = object(r#"{"model": "azure/gpt-4o"}"#);
    let mut keys = ModelCostKeys::default();
    let entry = cost_map_entry(&cost_map, &mut keys, &cipher(), &params, &object("{}")).await;
    assert!(entry.is_empty());
    assert_eq!(cost_map.lookups(), ["azure/gpt-4o"]);
}

#[rstest]
#[case::key_found(r#"{"key": "azure/gpt-4o"}"#, &["azure/gpt-4o"][..], r#"{"input_cost_per_token": 0.0000025}"#)]
#[case::key_unknown(r#"{"key": "azure/other"}"#, &["azure/other"][..], "{}")]
#[case::key_not_text(r#"{"key": 4}"#, &[][..], "{}")]
#[case::key_null(r#"{"key": null}"#, &[][..], "{}")]
#[case::no_key(r#"{"base_model": "azure/gpt-4o"}"#, &[][..], "{}")]
fn loaded_catalog_price(#[case] incoming: &str, #[case] lookups: &[&str], #[case] expected: &str) {
    let catalog = fake_cost_map(
        &[("azure/gpt-4o", r#"{"input_cost_per_token": 0.0000025}"#)],
        false,
    );
    assert_eq!(
        loaded_catalog_entry(&object(incoming), &catalog),
        object(expected)
    );
    assert_eq!(catalog.lookups(), lookups);
}
