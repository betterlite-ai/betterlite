use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::key_delegation::error::{DelegationDetail, DelegationError};
use crate::key_delegation::routes::{
    check_routes_permission, enforce_routes_update, is_safe_route_transition,
};

use super::{admin, json, presets, sent, user};

fn transition(incoming: &[&str], existing: &[&str]) -> bool {
    let incoming = presets(incoming);
    let existing = presets(existing);
    is_safe_route_transition(Some(&incoming), Some(&existing))
}

#[rstest]
#[case::null(None)]
#[case::empty(Some(Vec::new()))]
fn routes_omitted_falsey(#[case] value: Option<Vec<JsonData>>) {
    assert_eq!(
        check_routes_permission(&sent(false, value.clone()), false, user()),
        Ok(())
    );
    assert_eq!(
        check_routes_permission(&sent(false, value), true, user()),
        Ok(())
    );
}

#[rstest]
#[case::null_raw(None, false)]
#[case::null_derived(None, true)]
#[case::empty_raw(Some(Vec::new()), false)]
#[case::empty_derived(Some(Vec::new()), true)]
fn routes_explicit_null_or_empty_denied(
    #[case] value: Option<Vec<JsonData>>,
    #[case] allow_safe_presets: bool,
) {
    let error =
        check_routes_permission(&sent(true, value), allow_safe_presets, user()).unwrap_err();
    assert_eq!(error, DelegationError::AllowedRoutes);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert!(
        matches!(error.detail(), DelegationDetail::Error(message) if message.contains("key_type"))
    );
}

#[test]
fn routes_admin_bypass() {
    let custom = sent(true, Some(presets(&["management_routes"])));
    assert_eq!(check_routes_permission(&custom, false, admin()), Ok(()));
    assert_eq!(
        check_routes_permission(&sent(true, None), true, admin()),
        Ok(())
    );
    assert_eq!(
        check_routes_permission(&sent(true, Some(Vec::new())), false, admin()),
        Ok(())
    );
}

#[rstest]
#[case::single(&["info_routes"])]
#[case::both(&["llm_api_routes", "info_routes"])]
#[case::duplicates(&["llm_api_routes", "info_routes", "llm_api_routes"])]
fn routes_derived_safe_presets(#[case] routes: &[&str]) {
    assert_eq!(
        check_routes_permission(&sent(false, Some(presets(routes))), true, user()),
        Ok(())
    );
    // the raw body flow reports the same values as provided, so a non-admin may not send them
    assert_eq!(
        check_routes_permission(&sent(true, Some(presets(routes))), false, user()),
        Err(DelegationError::AllowedRoutes)
    );
    // and the derived flow is the only one that reads presets as safe
    assert_eq!(
        check_routes_permission(&sent(false, Some(presets(routes))), false, user()),
        Err(DelegationError::AllowedRoutes)
    );
}

#[rstest]
#[case::management(presets(&["management_routes"]))]
#[case::mixed(presets(&["llm_api_routes", "management_routes"]))]
#[case::case_mismatch(presets(&["info_routes", "Info_routes"]))]
#[case::non_string(vec![json("7")])]
fn routes_custom_or_mixed_denied(#[case] routes: Vec<JsonData>) {
    let error = check_routes_permission(&sent(false, Some(routes)), true, user()).unwrap_err();
    assert_eq!(error, DelegationError::AllowedRoutes);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert!(matches!(error.detail(), DelegationDetail::Error(_)));
}

#[test]
fn nested_route_entries_rejected() {
    let nested = [json("[1]"), json(r#"{"bucket": "info_routes"}"#)];
    assert_eq!(
        check_routes_permission(&sent(false, Some(nested.to_vec())), true, user()),
        Err(DelegationError::AllowedRoutes)
    );
    assert_eq!(
        check_routes_permission(&sent(true, Some(nested.to_vec())), true, user()),
        Err(DelegationError::AllowedRoutes)
    );
}

#[test]
fn nested_route_entries_unsafe_transition() {
    let nested = [json("[1]")];
    let read_only = presets(&["info_routes"]);
    assert!(!is_safe_route_transition(Some(&nested), Some(&read_only)));
    assert!(!is_safe_route_transition(Some(&read_only), Some(&nested)));
    // the fallback runs the raw check, which refuses a supplied nested list
    assert_eq!(
        enforce_routes_update(&sent(true, Some(nested.to_vec())), Some(&read_only), user()),
        Err(DelegationError::AllowedRoutes)
    );
}

#[rstest]
#[case::empty_to_empty(&[], &[], true)]
#[case::full_to_info(&["info_routes"], &[], true)]
#[case::full_to_llm(&["llm_api_routes"], &[], true)]
#[case::full_to_both(&["info_routes", "llm_api_routes"], &[], true)]
#[case::read_only_to_full(&[], &["info_routes"], false)]
#[case::read_only_to_llm(&["llm_api_routes"], &["info_routes"], false)]
#[case::read_only_to_both(&["llm_api_routes", "info_routes"], &["info_routes"], false)]
#[case::read_only_stays(&["info_routes"], &["info_routes"], true)]
#[case::llm_to_full(&[], &["llm_api_routes"], true)]
#[case::llm_to_info(&["info_routes"], &["llm_api_routes"], true)]
#[case::llm_to_both(&["info_routes", "llm_api_routes"], &["llm_api_routes"], true)]
#[case::incoming_custom(&["management_routes"], &[], false)]
#[case::existing_custom(&[], &["management_routes"], false)]
#[case::existing_mixed(&["info_routes"], &["info_routes", "management_routes"], false)]
#[case::custom_stays(&["management_routes"], &["management_routes"], false)]
fn preset_transition_matrix(
    #[case] incoming: &[&str],
    #[case] existing: &[&str],
    #[case] expected: bool,
) {
    assert_eq!(
        transition(incoming, existing),
        expected,
        "{incoming:?} -> {existing:?}"
    );
    let empty: [JsonData; 0] = [];
    let read_only = presets(&["info_routes"]);
    let custom = [json("7")];
    assert!(is_safe_route_transition(None, None));
    assert!(is_safe_route_transition(None, Some(empty.as_slice())));
    assert!(is_safe_route_transition(Some(empty.as_slice()), None));
    assert!(!is_safe_route_transition(None, Some(&read_only)));
    assert!(is_safe_route_transition(Some(&read_only), None));
    assert!(!is_safe_route_transition(None, Some(&custom)));
    assert!(!is_safe_route_transition(Some(&custom), None));
}

#[test]
fn preset_duplicates_order_ignored() {
    assert!(transition(
        &["info_routes", "info_routes"],
        &["info_routes"]
    ));
    assert!(transition(
        &["info_routes"],
        &["info_routes", "info_routes"]
    ));
    assert!(transition(
        &["llm_api_routes", "info_routes"],
        &["info_routes", "llm_api_routes"]
    ));
    assert!(transition(
        &["info_routes", "info_routes"],
        &["info_routes", "info_routes"]
    ));
    assert!(!transition(
        &["info_routes", "llm_api_routes"],
        &["info_routes", "info_routes"]
    ));
}

#[rstest]
#[case::null(None)]
#[case::empty(Some(Vec::new()))]
fn unsafe_transition_omitted_falsey_passthrough(#[case] value: Option<Vec<JsonData>>) {
    let existing = presets(&["info_routes"]);
    assert!(!is_safe_route_transition(value.as_deref(), Some(&existing)));
    // the field is absent, so the request default flows through and a non-admin passes the raw check
    assert_eq!(
        enforce_routes_update(&sent(false, value.clone()), Some(&existing), user()),
        Ok(())
    );
    // the same falsey value sent explicitly is refused, and an admin may still send it
    assert_eq!(
        enforce_routes_update(&sent(true, value), Some(&existing), user()),
        Err(DelegationError::AllowedRoutes)
    );
    assert_eq!(
        enforce_routes_update(&sent(true, None), Some(&existing), admin()),
        Ok(())
    );
    // a safe transition returns before the raw check, which would refuse the provided field
    assert_eq!(
        enforce_routes_update(&sent(true, Some(existing.clone())), Some(&existing), user()),
        Ok(())
    );
    // a custom existing restriction is never a safe transition
    let custom = presets(&["management_routes"]);
    assert_eq!(
        enforce_routes_update(
            &sent(true, Some(presets(&["info_routes"]))),
            Some(&custom),
            user()
        ),
        Err(DelegationError::AllowedRoutes)
    );
}
