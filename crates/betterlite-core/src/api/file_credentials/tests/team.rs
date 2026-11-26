use std::sync::Mutex;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::file_credentials::team::{
    apply_team_provider_credentials, get_team_provider_credentials,
};

use super::support::{
    Catalog, Log, LookupCall, NoProviders, ScriptedProviders, credentials, expansion, locked,
    lookup_calls, object, principal, row, team_catalog,
};

#[test]
fn team_credential_search_prefers_the_team_byok_row() {
    let events = Log::default();
    let catalog = team_catalog(&events);
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&[], &[], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(
            r#"{"api_key":"k1","api_base":"https://one","custom_llm_provider":"openai"}"#
        ))
    );
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "d1".to_owned(),
            team_id: Some("t".to_owned()),
        }]
    );
}

#[test]
fn team_credential_search_restricted_key_skips_a_row_it_cannot_use() {
    let events = Log::default();
    let catalog = team_catalog(&events);
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&["b"], &[], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(
            r#"{"api_key":"k2","api_base":"https://two","custom_llm_provider":"openai"}"#
        ))
    );
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "d2".to_owned(),
            team_id: Some("t".to_owned()),
        }]
    );
}

#[test]
fn team_credential_search_checks_the_public_name_not_the_model_name() {
    let events = Log::default();
    let catalog = Catalog {
        deployments: vec![row(
            "internal-a",
            r#"{"id":"d1","team_id":"t","team_public_model_name":"a"}"#,
        )],
        credentials: credentials(&[("d1", r#"{"api_key":"k1","custom_llm_provider":"openai"}"#)]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&["a"], &[], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(r#"{"api_key":"k1","custom_llm_provider":"openai"}"#))
    );
}

#[test]
fn team_credential_search_skips_other_teams_missing_ids_and_other_providers() {
    let events = Log::default();
    let catalog = team_catalog(&events);
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&[], &[], Some("t")),
        "azure",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(found, None);
    assert_eq!(
        lookup_calls(&catalog),
        [
            LookupCall {
                model_id: "d1".to_owned(),
                team_id: Some("t".to_owned()),
            },
            LookupCall {
                model_id: "d2".to_owned(),
                team_id: Some("t".to_owned()),
            },
            LookupCall {
                model_id: "d4".to_owned(),
                team_id: Some("t".to_owned()),
            },
        ]
    );
}

#[test]
fn provider_grant_fallback_tries_the_expanded_key_models_first() {
    let events = Log::default();
    let catalog = Catalog {
        model_names: vec!["gpt-4o".to_owned()],
        credentials: credentials(&[(
            "gpt-4o",
            r#"{"api_key":"kg","custom_llm_provider":"openai"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&["gpt-4o"], &[], None),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(r#"{"api_key":"kg","custom_llm_provider":"openai"}"#))
    );
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "gpt-4o".to_owned(),
            team_id: None,
        }]
    );
}

#[test]
fn provider_grant_fallback_uses_the_team_models_without_key_models() {
    let events = Log::default();
    let catalog = Catalog {
        credentials: credentials(&[(
            "claude-x",
            r#"{"api_key":"kc","custom_llm_provider":"anthropic"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&[], &["claude-x"], Some("t")),
        "anthropic",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(
            r#"{"api_key":"kc","custom_llm_provider":"anthropic"}"#
        ))
    );
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "claude-x".to_owned(),
            team_id: Some("t".to_owned()),
        }]
    );
    assert_eq!(locked(&catalog.name_calls).clone(), [Some("t".to_owned())]);
}

#[test]
fn provider_grant_fallback_reads_all_proxy_models_as_the_team_scoped_list() {
    let events = Log::default();
    let catalog = Catalog {
        model_names: vec!["gpt-4o".to_owned()],
        credentials: credentials(&[(
            "gpt-4o",
            r#"{"api_key":"kg","custom_llm_provider":"openai"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&[], &["all-proxy-models"], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(r#"{"api_key":"kg","custom_llm_provider":"openai"}"#))
    );
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "gpt-4o".to_owned(),
            team_id: Some("t".to_owned()),
        }]
    );
}

#[rstest]
#[case::all_proxy("all-proxy-models")]
#[case::all_team("all-team-models")]
fn provider_grant_fallback_sentinels_do_not_narrow_the_team(#[case] sentinel: &str) {
    let events = Log::default();
    let catalog = Catalog {
        model_names: vec!["proxy-model".to_owned()],
        deployments: vec![row(
            "public-x",
            r#"{"id":"d1","team_id":"t","team_public_model_name":"public-x"}"#,
        )],
        credentials: credentials(&[(
            "d1",
            r#"{"api_key":"k-byok","custom_llm_provider":"openai"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&[sentinel], &["claude-x"], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(
            r#"{"api_key":"k-byok","custom_llm_provider":"openai"}"#
        ))
    );
}

#[test]
fn provider_grant_fallback_expands_access_groups() {
    let events = Log::default();
    let catalog = Catalog {
        model_names: vec!["gpt-4o".to_owned()],
        access_groups: IndexMap::from([("group-a".to_owned(), vec!["gpt-4o".to_owned()])]),
        credentials: credentials(&[(
            "gpt-4o",
            r#"{"api_key":"kg","custom_llm_provider":"openai"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&["group-a"], &[], None),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(r#"{"api_key":"kg","custom_llm_provider":"openai"}"#))
    );
}

#[test]
fn provider_grant_fallback_expands_wildcards() {
    let events = Log::default();
    let catalog = Catalog {
        credentials: credentials(&[(
            "openai/gpt-4o",
            r#"{"api_key":"kw","custom_llm_provider":"openai"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = ScriptedProviders {
        provider: "openai".to_owned(),
        models: vec!["gpt-4o".to_owned()],
        asked: Mutex::new(Vec::new()),
    };
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&["openai/*"], &[], None),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(r#"{"api_key":"kw","custom_llm_provider":"openai"}"#))
    );
    assert_eq!(
        locked(&providers.asked).clone(),
        [Some("openai".to_owned())]
    );
    assert_eq!(
        lookup_calls(&catalog)
            .into_iter()
            .map(|call| call.model_id)
            .collect::<Vec<_>>(),
        ["openai/*", "openai/gpt-4o"]
    );
}

#[test]
fn provider_grant_fallback_empty_expanded_set_leaves_the_team_unrestrained() {
    let events = Log::default();
    let catalog = Catalog {
        access_groups: IndexMap::from([("empty-group".to_owned(), Vec::new())]),
        deployments: vec![row(
            "public-x",
            r#"{"id":"d1","team_id":"t","team_public_model_name":"public-x"}"#,
        )],
        credentials: credentials(&[(
            "d1",
            r#"{"api_key":"k-byok","custom_llm_provider":"openai"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let found = get_team_provider_credentials(
        Some(&catalog),
        &principal(&["empty-group"], &[], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        found,
        Some(object(
            r#"{"api_key":"k-byok","custom_llm_provider":"openai"}"#
        ))
    );
}

#[test]
fn provider_grant_fallback_without_a_match_yields_none() {
    let events = Log::default();
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let catalog = Catalog {
        credentials: credentials(&[(
            "other",
            r#"{"api_key":"ko","custom_llm_provider":"anthropic"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let identity = principal(&["other"], &[], None);
    assert_eq!(
        get_team_provider_credentials(
            None::<&Catalog>,
            &identity,
            "openai",
            &expansion(&providers, &snapshot)
        )
        .unwrap(),
        None
    );
    assert_eq!(
        get_team_provider_credentials(
            Some(&catalog),
            &identity,
            "openai",
            &expansion(&providers, &snapshot)
        )
        .unwrap(),
        None
    );
}

#[test]
fn provider_grant_fallback_apply_merges_the_provider_credentials() {
    let events = Log::default();
    let catalog = Catalog {
        deployments: vec![row(
            "a",
            r#"{"id":"d1","team_id":"t","team_public_model_name":"a"}"#,
        )],
        credentials: credentials(&[(
            "d1",
            r#"{"api_key":"k1","api_base":"https://one","custom_llm_provider":"openai","model":"m1"}"#,
        )]),
        events: events.clone(),
        ..Catalog::default()
    };
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let mut data = object(r#"{"model":"a","purpose":"batch"}"#);
    apply_team_provider_credentials(
        &mut data,
        Some(&catalog),
        &principal(&[], &[], Some("t")),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(
        data,
        object(r#"{"model":"a","purpose":"batch","api_key":"k1","api_base":"https://one"}"#)
    );
}

#[test]
fn provider_grant_fallback_apply_leaves_data_untouched() {
    let events = Log::default();
    let providers = NoProviders;
    let snapshot = CredentialSnapshot::default();
    let catalog = Catalog {
        events: events.clone(),
        ..Catalog::default()
    };
    let mut data = object(r#"{"model":"a","purpose":"batch"}"#);
    let before = data.clone();
    apply_team_provider_credentials(
        &mut data,
        Some(&catalog),
        &principal(&[], &[], None),
        "openai",
        &expansion(&providers, &snapshot),
    )
    .unwrap();
    assert_eq!(data, before);
}
