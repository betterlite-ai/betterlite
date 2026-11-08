use rstest::rstest;

use super::support::{
    BASE, KEY, ResponseKind, Scenario, answer, failing, json, log_named_projects, logger,
    model_completion, slow_answer,
};
use crate::braintrust_logging::logger::BraintrustError;
use crate::braintrust_logging::payload::CallbackPath;

fn named_project() -> Scenario {
    let mut scenario = Scenario::empty();
    scenario.response_kind = ResponseKind::Model;
    scenario.response_body = model_completion();
    scenario.metadata = json(r#"{"project_name":"analytics"}"#);
    scenario
}

#[tokio::test]
async fn project_resolution_posts_to_the_sync_endpoint_and_caches_the_id() {
    let (logger, replay) = logger(
        vec![answer(r#"{"id":"proj-1"}"#), answer("{}"), answer("{}")],
        &[],
    );
    let scenario = named_project();
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url, format!("{BASE}/project"));
    assert_eq!(requests[0].body, json(r#"{"name":"analytics"}"#));
    assert_eq!(
        requests[0].headers["authorization"],
        format!("Bearer {KEY}")
    );
    assert!(requests[0].sensitive.contains("authorization"));
    assert_eq!(requests[0].headers["content-type"], "application/json");
    assert_eq!(
        requests[1].url,
        format!("{BASE}/project_logs/proj-1/insert")
    );
    assert_eq!(requests[2].url, requests[1].url);
}

#[tokio::test]
async fn project_resolution_posts_to_the_register_endpoint_on_the_async_path() {
    let (logger, replay) = logger(vec![answer(r#"{"id":"proj-2"}"#), answer("{}")], &[]);
    logger
        .log_success_event(&named_project().event(), CallbackPath::Async)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests[0].url, format!("{BASE}/project/register"));
    assert_eq!(
        requests[1].url,
        format!("{BASE}/project_logs/proj-2/insert")
    );
}

#[tokio::test]
async fn project_resolution_prefers_a_metadata_project_id() {
    let (logger, replay) = logger(vec![answer("{}")], &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_id":"direct","project_name":"analytics"}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        format!("{BASE}/project_logs/direct/insert")
    );
}

#[tokio::test]
async fn project_resolution_retries_a_lookup_that_failed() {
    let (logger, replay) = logger(
        vec![
            failing(500, r#"{"error":"boom"}"#),
            answer(r#"{"id":"proj-3"}"#),
            answer("{}"),
        ],
        &[],
    );
    let scenario = named_project();
    let failure = logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        BraintrustError::Status { status: 500, .. }
    ));
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url, format!("{BASE}/project"));
    assert_eq!(requests[1].url, format!("{BASE}/project"));
    assert_eq!(
        requests[2].url,
        format!("{BASE}/project_logs/proj-3/insert")
    );
}

#[tokio::test]
async fn project_resolution_keeps_the_response_text() {
    let (logger, _replay) = logger(vec![failing(400, r#"{"error":"bad request"}"#)], &[]);
    let failure = logger
        .log_success_event(&named_project().event(), CallbackPath::Sync)
        .await
        .unwrap_err();
    let BraintrustError::Status { status, body } = failure else {
        panic!("expected a status failure");
    };
    assert_eq!(status, 400);
    assert_eq!(body, r#"{"error":"bad request"}"#);
}

#[rstest]
#[case::dot(".")]
#[case::dot_dot("..")]
#[tokio::test]
async fn dot_segment_project_ids_are_rejected(#[case] project_id: &str) {
    let (logger, replay) = logger(Vec::new(), &[]);
    let mut scenario = Scenario::empty();
    scenario.metadata = json(&format!(r#"{{"project_id":"{project_id}"}}"#));
    let failure = logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap_err();
    assert!(matches!(failure, BraintrustError::PathSegment));
    assert!(replay.requests().is_empty());
}

#[tokio::test]
async fn project_cache_keeps_1024_names_and_evicts_the_oldest() {
    let answers = (0..1025_u32)
        .flat_map(|index| [answer(&format!(r#"{{"id":"p{index}"}}"#)), answer("{}")])
        .chain([answer(r#"{"id":"p0"}"#), answer("{}")])
        .chain([answer(r#"{"id":"p1"}"#), answer("{}")])
        .chain([answer(r#"{"id":"p2"}"#), answer("{}")])
        .collect::<Vec<_>>();
    let (logger, replay) = logger(answers, &[]);
    let names = (0..1025_u32)
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>();
    log_named_projects(&logger, &names).await.unwrap();
    assert_eq!(replay.requests().len(), 2050);
    log_named_projects(&logger, &names[..1]).await.unwrap();
    let after_p0 = replay.requests();
    assert_eq!(after_p0.len(), 2052);
    assert_eq!(after_p0[2050].url, format!("{BASE}/project"));
    assert_eq!(after_p0[2050].body, json(r#"{"name":"p0"}"#));
    assert!(after_p0[2051].url.ends_with("/project_logs/p0/insert"));
    log_named_projects(&logger, &names[1..2]).await.unwrap();
    let after_p1 = replay.requests();
    assert_eq!(after_p1.len(), 2054);
    assert_eq!(after_p1[2052].url, format!("{BASE}/project"));
    assert_eq!(after_p1[2052].body, json(r#"{"name":"p1"}"#));
    assert!(after_p1[2053].url.ends_with("/project_logs/p1/insert"));
    log_named_projects(&logger, &names[2..3]).await.unwrap();
    let after_p2 = replay.requests();
    assert_eq!(after_p2.len(), 2056);
    assert_eq!(after_p2[2054].url, format!("{BASE}/project"));
    assert_eq!(after_p2[2054].body, json(r#"{"name":"p2"}"#));
    assert!(after_p2[2055].url.ends_with("/project_logs/p2/insert"));
}

#[tokio::test]
async fn duplicate_project_inserts_do_not_grow_the_cache_past_1024() {
    let mut answers = vec![
        slow_answer(r#"{"id":"dup-a"}"#),
        slow_answer(r#"{"id":"dup-b"}"#),
        answer("{}"),
        answer("{}"),
    ];
    answers.extend(
        (0..1025_u32).flat_map(|index| [answer(&format!(r#"{{"id":"n{index}"}}"#)), answer("{}")]),
    );
    answers.extend([answer(r#"{"id":"n0"}"#), answer("{}")]);
    let (logger, replay) = logger(answers, &[]);
    let first = named_project();
    let second = named_project();
    let first_event = first.event();
    let second_event = second.event();
    let (left, right) = tokio::join!(
        logger.log_success_event(&first_event, CallbackPath::Sync),
        logger.log_success_event(&second_event, CallbackPath::Sync),
    );
    left.unwrap();
    right.unwrap();
    assert_eq!(replay.requests().len(), 4);
    let names = (0..1025_u32)
        .map(|index| format!("n{index}"))
        .collect::<Vec<_>>();
    log_named_projects(&logger, &names).await.unwrap();
    assert_eq!(replay.requests().len(), 2054);
    log_named_projects(&logger, &names[..1]).await.unwrap();
    let requests = replay.requests();
    assert_eq!(requests.len(), 2056);
    assert_eq!(requests[2054].url, format!("{BASE}/project"));
    assert_eq!(requests[2054].body, json(r#"{"name":"n0"}"#));
    assert_eq!(requests[2055].url, format!("{BASE}/project_logs/n0/insert"));
}
