use super::support::{Fixture, deployment, healthy, unhealthy};
use crate::health::router::{
    Endpoint, dependency_deployments_to_probe, finalize_strategy_router_endpoints,
    strategy_router_verdicts,
};

fn tiers(name: &str) -> String {
    format!(
        r#"{{"model":"auto_router/complexity_router","complexity_router_config":{{"tiers":{{"SIMPLE":"{name}"}}}}}}"#
    )
}

#[test]
fn strategy_router_verdicts_behaviors() {
    assert_settled_through_a_child_router();
    assert_expansion_reaches_through_routers();
    assert_cycle_terminates();
}

// The child reds on its own missing tier, and the parent reds only in the next round, once that verdict feeds back in.
fn assert_settled_through_a_child_router() {
    let chain = vec![
        deployment("parent", "parent-id", &tiers("child")),
        deployment("child", "child-id", &tiers("missing-leaf")),
    ];
    let fixture = Fixture::new(&chain);
    let lookup = fixture.lookup();
    let verdicts = strategy_router_verdicts(
        &[healthy("parent-id"), healthy("child-id")],
        &[],
        &chain,
        &lookup,
    )
    .unwrap();
    assert_eq!(
        verdicts.get("child-id").map(String::as_str),
        Some("tier model 'missing-leaf' matches no deployment on this proxy")
    );
    assert_eq!(
        verdicts.get("parent-id").map(String::as_str),
        Some("tier model 'child' has no healthy deployment")
    );
}

// One hop per round follows routers through routers, so the grandchild is probed for the grandparent to red.
fn assert_expansion_reaches_through_routers() {
    let rows = vec![
        deployment("outer", "outer-id", &tiers("middle")),
        deployment("middle", "middle-id", &tiers("leaf")),
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let expanded = dependency_deployments_to_probe(&rows[..1], &rows, &lookup).unwrap();
    let expanded_names: Vec<&str> = expanded.iter().map(|row| row.model_name.as_str()).collect();
    assert_eq!(expanded_names, ["middle", "leaf"]);
}

// A cycle with no fault settles green, and a cycle where one router has a missing tier reds both, one round apart, and stops.
fn assert_cycle_terminates() {
    assert_clean_cycle_stays_green();
    assert_faulted_cycle_reds_both();
}

fn assert_clean_cycle_stays_green() {
    let clean = vec![
        deployment("a", "a-id", &tiers("b")),
        deployment("b", "b-id", &tiers("a")),
    ];
    let clean_fixture = Fixture::new(&clean);
    let clean_lookup = clean_fixture.lookup();
    assert!(
        strategy_router_verdicts(
            &[healthy("a-id"), healthy("b-id")],
            &[],
            &clean,
            &clean_lookup
        )
        .unwrap()
        .is_empty()
    );
}

fn assert_faulted_cycle_reds_both() {
    let cycle = vec![
        deployment("a", "a-id", &tiers("b")),
        deployment(
            "b",
            "b-id",
            r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"a","COMPLEX":"missing-leaf"}}}"#,
        ),
    ];
    let cycle_fixture = Fixture::new(&cycle);
    let cycle_lookup = cycle_fixture.lookup();
    let verdicts = strategy_router_verdicts(
        &[healthy("a-id"), healthy("b-id")],
        &[],
        &cycle,
        &cycle_lookup,
    )
    .unwrap();
    assert_eq!(verdicts.len(), 2);
    assert_eq!(
        verdicts.get("a-id").map(String::as_str),
        Some("tier model 'b' has no healthy deployment")
    );
    assert_eq!(
        verdicts.get("b-id").map(String::as_str),
        Some("tier model 'missing-leaf' matches no deployment on this proxy")
    );
}

#[test]
fn strategy_router_verdicts_boundaries() {
    assert_default_role_message();
    assert_replica_evidence();
    assert_unusable_ids_are_no_fault();
    assert_keyed_order();
    assert_evaluation_dependency_ignored();
    assert_plain_deployment_gets_no_verdict();
    assert_unknown_endpoints_ignored();
}

// The default role names itself in the message, apart from the tier role the other cases carry.
fn assert_default_role_message() {
    let rows = vec![deployment(
        "router",
        "router-id",
        r#"{"model":"auto_router/complexity_router","complexity_router_default_model":"missing-default"}"#,
    )];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let verdicts = strategy_router_verdicts(&[healthy("router-id")], &[], &rows, &lookup).unwrap();
    assert_eq!(
        verdicts.get("router-id").map(String::as_str),
        Some("default model 'missing-default' matches no deployment on this proxy")
    );
}

// Two deployments behind one name: one unhealthy replica leaves the verdict green, every replica unhealthy reds it.
fn assert_replica_evidence() {
    let rows = vec![
        deployment("router", "router-id", &tiers("leaf")),
        deployment("leaf", "leaf-a", r#"{"model":"openai/leaf"}"#),
        deployment("leaf", "leaf-b", r#"{"model":"openai/leaf"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    assert!(
        strategy_router_verdicts(
            &[healthy("router-id")],
            &[unhealthy("leaf-a")],
            &rows,
            &lookup
        )
        .unwrap()
        .is_empty()
    );
    let verdicts = strategy_router_verdicts(
        &[healthy("router-id")],
        &[unhealthy("leaf-a"), unhealthy("leaf-b")],
        &rows,
        &lookup,
    )
    .unwrap();
    assert_eq!(
        verdicts.get("router-id").map(String::as_str),
        Some("tier model 'leaf' has no healthy deployment")
    );
}

// A dependency whose name lists only rows without a usable id is no fault, since no probe can judge it.
fn assert_unusable_ids_are_no_fault() {
    let rows = vec![
        deployment("router", "router-id", &tiers("blank")),
        deployment("blank", "", r#"{"model":"openai/blank"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    assert!(
        strategy_router_verdicts(&[healthy("router-id")], &[], &rows, &lookup)
            .unwrap()
            .is_empty()
    );
}

// Verdicts are keyed by id and come out in id order, and a router reported twice yields one verdict.
fn assert_keyed_order() {
    let rows = vec![
        deployment("beta", "beta-id", &tiers("missing-b")),
        deployment("alpha", "alpha-id", &tiers("missing-a")),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let verdicts = strategy_router_verdicts(
        &[healthy("beta-id"), healthy("alpha-id"), healthy("alpha-id")],
        &[],
        &rows,
        &lookup,
    )
    .unwrap();
    assert_eq!(
        verdicts.keys().cloned().collect::<Vec<String>>(),
        ["alpha-id".to_owned(), "beta-id".to_owned()]
    );
}

// The evaluation classifier's bundled model is never a fault.
fn assert_evaluation_dependency_ignored() {
    let rows = vec![deployment(
        "router",
        "router-id",
        r#"{"model":"auto_router/complexity_router","complexity_router_config":{"classifier_type":"jev"}}"#,
    )];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    assert!(
        strategy_router_verdicts(&[healthy("router-id")], &[], &rows, &lookup)
            .unwrap()
            .is_empty()
    );
}

// A plain deployment carries no dependencies, so a healthy probe of it gains no verdict whatever it is called.
fn assert_plain_deployment_gets_no_verdict() {
    let rows = vec![deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#)];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    assert!(
        strategy_router_verdicts(&[healthy("leaf-id")], &[], &rows, &lookup)
            .unwrap()
            .is_empty()
    );
}

// Endpoints the checked set does not know, and endpoints without an id, are ignored on both sides.
fn assert_unknown_endpoints_ignored() {
    let rows = vec![deployment("router", "router-id", &tiers("missing"))];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let unknown = Endpoint {
        model_id: Some("unlisted-id".to_owned()),
        error: None,
    };
    let anonymous = Endpoint {
        model_id: None,
        error: Some("the probe failed".to_owned()),
    };
    let verdicts = strategy_router_verdicts(&[unknown], &[anonymous], &rows, &lookup).unwrap();
    assert!(verdicts.is_empty());
}

#[test]
fn finalize_applies_verdicts_and_drops_dependency_probes() {
    assert_verdicts_move_and_probes_drop();
    assert_without_router_keeps_requested();
    assert_anonymous_endpoint_kept();
}

// Probed-only deployments drop out of both sides, and the router moves behind the kept unhealthy rows with its verdict.
fn assert_verdicts_move_and_probes_drop() {
    let rows = vec![
        deployment("router", "router-id", &tiers("bad")),
        deployment("bad", "bad-id", r#"{"model":"openai/bad"}"#),
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let probes = vec![rows[1].clone(), rows[2].clone()];
    let (kept_healthy, kept_unhealthy) = finalize_strategy_router_endpoints(
        &[healthy("router-id"), healthy("leaf-id")],
        &[unhealthy("stray-id"), unhealthy("bad-id")],
        &rows,
        Some(&lookup),
        &probes,
    )
    .unwrap();
    assert!(kept_healthy.is_empty());
    assert_eq!(
        kept_unhealthy,
        [
            unhealthy("stray-id"),
            Endpoint {
                model_id: Some("router-id".to_owned()),
                error: Some("tier model 'bad' has no healthy deployment".to_owned()),
            },
        ]
    );
}

// Without a router there are no verdicts, and the requested router stays healthy.
fn assert_without_router_keeps_requested() {
    let rows = vec![
        deployment("router", "router-id", &tiers("bad")),
        deployment("bad", "bad-id", r#"{"model":"openai/bad"}"#),
    ];
    let probes = vec![rows[1].clone()];
    let (kept_healthy, kept_unhealthy) =
        finalize_strategy_router_endpoints(&[healthy("router-id")], &[], &rows, None, &probes)
            .unwrap();
    assert_eq!(kept_healthy, [healthy("router-id")]);
    assert!(kept_unhealthy.is_empty());
}

// An endpoint without an id is never one of the dropped probes.
fn assert_anonymous_endpoint_kept() {
    let rows = vec![deployment("router", "router-id", &tiers("bad"))];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let anonymous = Endpoint {
        model_id: None,
        error: None,
    };
    let (kept_healthy, _) =
        finalize_strategy_router_endpoints(&[anonymous], &[], &rows, Some(&lookup), &[]).unwrap();
    assert_eq!(kept_healthy.len(), 1);
}
