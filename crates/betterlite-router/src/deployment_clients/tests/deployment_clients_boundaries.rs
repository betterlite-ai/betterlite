use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::{CLIENT_TTL_SECONDS, catalog, clock_at, limit, spec, system_clock, text};
use crate::deployment_clients::clients::{
    CachedClients, ClientDeployment, ClientKind, ClientLookup,
};
use crate::deployment_clients::guard::calculate_parallel_limit;
use crate::deployment_clients::lookup::{
    credential_deployment, deployment_credentials, provider_credentials,
};
use crate::deployment_clients::request::{DeploymentView, RequestContext};

// Bounds, error fields and exact selection: the values just inside and just outside each limit.
#[rstest]
#[case::first_slot_of_one(1, 0, true)]
#[case::last_slot_of_two(2, 1, true)]
#[case::at_the_limit(1, 1, false)]
#[case::at_the_limit_of_two(2, 2, false)]
#[case::zero_admits_nothing(0, 0, false)]
#[case::negative_admits_nothing(-1, 0, false)]
fn acquire_compares_against_the_cap(
    #[case] max_parallel_requests: i64,
    #[case] held_slots: i64,
    #[case] granted: bool,
) {
    let limit = limit(max_parallel_requests);
    let held: Vec<_> = (0..held_slots)
        .map(|_| limit.acquire().expect("within the cap"))
        .collect();
    let acquired = limit.acquire();
    let granted_now = acquired.is_ok();
    assert_eq!(granted_now, granted);
    assert_eq!(
        limit.in_flight.load(Ordering::SeqCst),
        held_slots + i64::from(granted_now)
    );
    drop(acquired);
    drop(held);
    assert_eq!(limit.in_flight.load(Ordering::SeqCst), 0);
}

#[rstest]
#[case::tpm_zero(Some(0), Some(1))]
#[case::tpm_negative_rounds_to_one(Some(-150), Some(1))]
#[case::tpm_negative_beyond_one(Some(-1500), Some(-9))]
#[case::tpm_exact_multiple(Some(5000), Some(30))]
fn tpm_truncates_like_the_azure_formula(#[case] tpm: Option<i64>, #[case] expected: Option<i64>) {
    assert_eq!(calculate_parallel_limit(None, None, tpm, None), expected);
}

#[test]
fn an_id_beats_a_name_and_a_name_beats_a_wildcard() {
    let by_id = spec("shared", "d1", &[("api_key", text("sk-id"))], &[]);
    let by_name = spec("shared", "d2", &[("api_key", text("sk-name"))], &[]);
    let wildcard = spec("openai/*", "d3", &[("api_key", text("sk-wild"))], &[]);
    let catalog = catalog(&[by_name, wildcard, by_id]);
    let by_id_pick = credential_deployment(&catalog, "d1", None).expect("by id");
    assert_eq!(by_id_pick.id(), Some("d1"));
    let by_name_pick = credential_deployment(&catalog, "shared", None).expect("by name");
    assert_eq!(by_name_pick.id(), Some("d2"));
    let wildcard_pick = credential_deployment(&catalog, "openai/gpt-4o", None).expect("wildcard");
    assert_eq!(wildcard_pick.id(), Some("d3"));
}

#[test]
fn another_teams_deployment_is_not_resolved_by_name() {
    let team_deployment = spec(
        "shared",
        "d1",
        &[("api_key", text("sk-team"))],
        &[("team_id", text("team-a"))],
    );
    let catalog = catalog(&[team_deployment]);
    assert!(credential_deployment(&catalog, "shared", None).is_none());
    assert!(credential_deployment(&catalog, "shared", Some("team-b")).is_none());
    let chosen = credential_deployment(&catalog, "shared", Some("team-a")).expect("own team");
    assert_eq!(chosen.id(), Some("d1"));
    assert!(credential_deployment(&catalog, "d1", Some("team-b")).is_some());
}

#[test]
fn a_team_wildcard_shadows_a_global_one() {
    let global = spec("openai/*", "g1", &[("api_key", text("sk-global"))], &[]);
    let team = spec(
        "openai/*",
        "t1",
        &[("api_key", text("sk-team"))],
        &[
            ("team_id", text("team-a")),
            ("team_public_model_name", text("openai/*")),
        ],
    );
    let catalog = catalog(&[global, team]);
    let own_team = credential_deployment(&catalog, "openai/gpt-4o", Some("team-a")).expect("team");
    assert_eq!(own_team.id(), Some("t1"));
    let other_team =
        credential_deployment(&catalog, "openai/gpt-4o", Some("team-b")).expect("global");
    assert_eq!(other_team.id(), Some("g1"));
    let no_team = credential_deployment(&catalog, "openai/gpt-4o", None).expect("global");
    assert_eq!(no_team.id(), Some("g1"));
}

#[test]
fn a_cached_client_expires_exactly_at_the_client_ttl() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let lookup = ClientLookup {
        model_id: "d1",
        stream: false,
        api_key: None,
    };
    let first = clients.openai_client(
        ClientKind::Async,
        &lookup,
        &ClientDeployment {
            model_group: "gpt",
            api_base: "https://a.example/v1",
        },
    );
    assert_eq!(first.api_base, "https://a.example/v1");
    *clock.lock().expect("clock") = 1;
    let fresh = clients.openai_client(
        ClientKind::Async,
        &lookup,
        &ClientDeployment {
            model_group: "gpt",
            api_base: "https://b.example/v1",
        },
    );
    assert_eq!(fresh.api_base, "https://a.example/v1");
    *clock.lock().expect("clock") = CLIENT_TTL_SECONDS.saturating_mul(1000);
    let expired = clients.openai_client(
        ClientKind::Async,
        &lookup,
        &ClientDeployment {
            model_group: "gpt",
            api_base: "https://c.example/v1",
        },
    );
    assert_eq!(expired.api_base, "https://c.example/v1");
}

#[test]
fn a_null_credential_field_is_dropped() {
    let built = spec("gpt", "d1", &[("api_key", Json::Null)], &[]);
    let catalog = catalog(&[built]);
    let credentials = deployment_credentials(&catalog, "d1")
        .expect("lookup")
        .expect("credentials");
    assert!(!credentials.contains_key("api_key"));
    assert!(credentials.is_empty());
}

#[test]
fn a_declared_provider_wins_over_the_model_prefix() {
    let built = spec(
        "claude",
        "d1",
        &[
            ("model", text("anthropic/claude-sonnet-5")),
            ("custom_llm_provider", text("bedrock")),
        ],
        &[],
    );
    let catalog = catalog(&[built]);
    let credentials =
        provider_credentials(&catalog, "d1", None, &std::collections::BTreeMap::new())
            .expect("lookup")
            .expect("credentials");
    assert_eq!(
        credentials.get("custom_llm_provider"),
        Some(&text("bedrock"))
    );
}

#[test]
fn an_empty_update_writes_no_tags() {
    let info = JsonObject::new();
    let mut context = RequestContext::default();
    context.update(&DeploymentView {
        model_name: "gpt",
        model: "openai/gpt-4o",
        api_base: None,
        model_info: &info,
        tags: &[],
        credential_name: None,
        tools: &[],
        tool_choice: None,
    });
    assert!(!context.metadata.contains_key("tags"));
    assert!(context.tags.is_empty());
}

#[test]
fn a_credential_field_of_the_wrong_type_is_an_error() {
    let flag = spec("gpt", "d1", &[("api_key", Json::Bool(true))], &[]);
    let error = deployment_credentials(&catalog(&[flag]), "d1").expect_err("a bool is no key");
    assert!(error.to_string().contains("api_key"), "{error}");
    let list = spec("gpt", "d2", &[("api_key", Json::List(Vec::new()))], &[]);
    assert!(deployment_credentials(&catalog(&[list]), "d2").is_err());
    let object = spec(
        "gpt",
        "d3",
        &[("vertex_credentials", Json::Object(JsonObject::new()))],
        &[],
    );
    assert!(deployment_credentials(&catalog(&[object]), "d3").is_ok());
    let tags = spec(
        "gpt",
        "d4",
        &[("bedrock_tags", Json::List(Vec::new()))],
        &[],
    );
    assert!(deployment_credentials(&catalog(&[tags]), "d4").is_ok());
}
