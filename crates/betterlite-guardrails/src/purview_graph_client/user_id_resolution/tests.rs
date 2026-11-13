#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use super::{KeyIdentity, ResolvedUserId, resolve_logging_user_id, resolve_user_id};
use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::test_support::guardrail;

fn data(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

fn base() -> PurviewGuardrailBase {
    guardrail("http://127.0.0.1:9")
}

fn trusted(user_id: &str) -> ResolvedUserId {
    ResolvedUserId::Trusted(user_id.to_owned())
}

fn untrusted(user_id: &str) -> ResolvedUserId {
    ResolvedUserId::Untrusted(user_id.to_owned())
}

#[rstest]
#[case::key_bound_user_wins(
    Some("key-user"),
    Some("end-user"),
    r#"{"metadata": {"user_api_key_user_id": "proxy-user", "user_id": "caller-user"}}"#,
    Some(trusted("key-user"))
)]
#[case::end_user_is_second(
    None,
    Some("end-user"),
    r#"{"metadata": {"user_api_key_user_id": "proxy-user"}}"#,
    Some(untrusted("end-user"))
)]
#[case::proxy_injected_metadata_is_third(
    None,
    None,
    r#"{"metadata": {"user_api_key_user_id": "proxy-user", "user_id": "caller-user"}}"#,
    Some(untrusted("proxy-user"))
)]
#[case::configured_metadata_field_is_last(
    None,
    None,
    r#"{"metadata": {"user_id": "caller-user"}}"#,
    Some(untrusted("caller-user"))
)]
#[case::litellm_metadata_is_the_fallback_container(
    None,
    None,
    r#"{"litellm_metadata": {"user_api_key_user_id": "proxy-user"}}"#,
    Some(untrusted("proxy-user"))
)]
#[case::empty_metadata_falls_through_to_litellm_metadata(
    None,
    None,
    r#"{"metadata": {}, "litellm_metadata": {"user_id": "caller-user"}}"#,
    Some(untrusted("caller-user"))
)]
#[case::empty_ids_count_as_missing(
    Some(""),
    Some(""),
    r#"{"metadata": {"user_api_key_user_id": "", "user_id": "caller-user"}}"#,
    Some(untrusted("caller-user"))
)]
#[case::metadata_that_is_not_an_object(None, None, r#"{"metadata": "x"}"#, None)]
#[case::no_identity_at_all(None, None, "{}", None)]
fn user_id_resolution_walks_the_sources_in_trust_order(
    #[case] user_id: Option<&str>,
    #[case] end_user_id: Option<&str>,
    #[case] request: &str,
    #[case] expected: Option<ResolvedUserId>,
) {
    let identity = KeyIdentity {
        user_id,
        end_user_id,
    };

    assert_eq!(
        resolve_user_id(&base(), &data(request), &identity),
        expected
    );
}

#[test]
fn user_id_resolution_reads_the_configured_metadata_field_name() {
    let mut guardrail = base();
    guardrail.user_id_field = "entra_object_id".to_owned();
    let request = data(r#"{"metadata": {"user_id": "ignored", "entra_object_id": "oid-1"}}"#);

    let resolved = resolve_user_id(&guardrail, &request, &KeyIdentity::default());

    assert_eq!(resolved, Some(untrusted("oid-1")));
}

#[rstest]
#[case::key_bound(
    Some("key-user"),
    r#"{"metadata": {"user_id": "caller"}}"#,
    Some("key-user")
)]
#[case::end_user_is_never_trusted(None, "{}", None)]
#[case::metadata_is_never_trusted(
    None,
    r#"{"metadata": {"user_api_key_user_id": "proxy-user", "user_id": "caller"}}"#,
    None
)]
fn user_id_resolution_trusts_only_the_key_bound_id_for_blocking(
    #[case] user_id: Option<&str>,
    #[case] request: &str,
    #[case] expected: Option<&str>,
) {
    let identity = KeyIdentity {
        user_id,
        end_user_id: Some("end-user"),
    };
    let resolved = resolve_user_id(&base(), &data(request), &identity);

    let trusted_id = resolved.and_then(ResolvedUserId::into_trusted);

    assert_eq!(trusted_id.as_deref(), expected);
}

#[rstest]
#[case::injected_metadata(
    r#"{"litellm_params": {"metadata": {"user_api_key_user_id": "proxy-user", "user_id": "caller"}}}"#,
    Some("proxy-user")
)]
#[case::top_level_kwarg(r#"{"user_api_key_user_id": "top-user"}"#, Some("top-user"))]
#[case::metadata_wins_over_the_kwarg(
    r#"{"litellm_params": {"metadata": {"user_api_key_user_id": "meta"}}, "user_api_key_user_id": "top"}"#,
    Some("meta")
)]
#[case::caller_supplied_ids_are_ignored(
    r#"{"litellm_params": {"metadata": {"user_id": "caller", "user_api_key_end_user_id": "end"}}, "user_api_key_end_user_id": "end"}"#,
    None
)]
#[case::metadata_that_is_not_an_object(r#"{"litellm_params": {"metadata": "x"}}"#, None)]
#[case::litellm_params_that_is_not_an_object(r#"{"litellm_params": []}"#, None)]
#[case::empty_id_is_missing(
    r#"{"litellm_params": {"metadata": {"user_api_key_user_id": ""}}}"#,
    None
)]
#[case::nothing("{}", None)]
fn user_id_resolution_reads_only_the_injected_id_from_logging_kwargs(
    #[case] kwargs: &str,
    #[case] expected: Option<&str>,
) {
    let resolved = resolve_logging_user_id(&data(kwargs));

    assert_eq!(resolved.as_deref(), expected);
}
