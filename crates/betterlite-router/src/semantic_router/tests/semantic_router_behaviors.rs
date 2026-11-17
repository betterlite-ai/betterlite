use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, Role};
use indexmap::IndexMap;
use rstest::rstest;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;

use crate::semantic_router::error::{EmbeddingStage, RouterError};
use crate::semantic_router::index::RouteIndexError;
use crate::semantic_router::service::{AutoRouter, AutoRouterSettings, CallerContext};
use crate::semantic_router::tests::support::{
    FakeEmbedder, TWO_ROUTES, caller, decisions, image_part, keyless_text_part, message,
    numeric_text_part, router, settings, temporary_directory, text_part, user_parts, user_text,
    with_limit,
};

fn embedding_arena() -> Vec<(&'static str, Vec<f64>)> {
    vec![
        ("test", vec![1.0, 0.0]),
        ("write code", vec![1.0, 0.0]),
        ("small talk", vec![0.0, 1.0]),
        ("fix it", vec![1.0, 0.0]),
    ]
}

#[tokio::test]
async fn a_matching_route_supplies_the_model() {
    let embedder = Arc::new(FakeEmbedder::new(&embedding_arena()));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .expect("the route index builds");
    assert_eq!(decision, decisions(Some("code"), "code"));
    let documents = embedder.document_calls();
    assert_eq!(documents[0].model, "fake-embed");
    assert_eq!(documents[0].texts, vec!["test"]);
    let query = embedder.query_calls();
    assert_eq!(query[0].model, "fake-embed");
    assert_eq!(query[0].text, "fix it");
}

#[tokio::test]
async fn no_route_above_its_threshold_falls_back_to_the_default_model() {
    let config =
        r#"{"routes": [{"name": "code", "utterances": ["write code"], "score_threshold": 0.9}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("write code", vec![0.0, 1.0]),
        ("fix it", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("fix it")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
}

#[tokio::test]
async fn a_failed_query_embedding_falls_back_to_the_default_model() {
    let embedder = Arc::new(FakeEmbedder::new(&embedding_arena()).failing_queries(1));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
    assert_eq!(embedder.query_calls().len(), 1);
}

#[tokio::test]
async fn a_failed_build_reaches_the_caller() {
    let embedder = Arc::new(FakeEmbedder::new(&[]).failing_documents(1));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    let error = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .expect_err("a failed build is not a routing fallback");
    assert!(matches!(
        error.as_ref(),
        RouterError::Embedding {
            stage: EmbeddingStage::Documents,
            ..
        }
    ));
}

#[tokio::test]
async fn a_failed_build_is_retried_by_the_next_call() {
    let embedder = Arc::new(FakeEmbedder::new(&embedding_arena()).failing_documents(1));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    assert!(
        router
            .decide_with(Some(&messages), || Some(caller()))
            .await
            .is_err()
    );
    let second = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(second, decisions(Some("code"), "code"));
    assert_eq!(embedder.document_calls().len(), 3);
}

#[tokio::test]
async fn concurrent_calls_share_one_index_build() {
    let embedder = Arc::new(FakeEmbedder::new(&embedding_arena()));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    let (first, second, third, fourth) = tokio::join!(
        router.decide_with(Some(&messages), || Some(caller())),
        router.decide_with(Some(&messages), || Some(caller())),
        router.decide_with(Some(&messages), || Some(caller())),
        router.decide_with(Some(&messages), || Some(caller())),
    );
    assert_eq!(first.unwrap(), decisions(Some("code"), "code"));
    assert_eq!(second.unwrap(), decisions(Some("code"), "code"));
    assert_eq!(third.unwrap(), decisions(Some("code"), "code"));
    assert_eq!(fourth.unwrap(), decisions(Some("code"), "code"));
    assert_eq!(embedder.document_calls().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_index_build_runs_on_the_supplied_task_tracker() {
    let tasks = TaskTracker::new();
    let embedder =
        Arc::new(FakeEmbedder::new(&embedding_arena()).with_delay(Duration::from_millis(10)));
    let router = Arc::new(
        AutoRouter::new(settings(TWO_ROUTES), Arc::clone(&embedder), tasks.clone())
            .expect("the route config parses"),
    );
    let messages = vec![user_text("fix it")];
    let caller_router = Arc::clone(&router);
    let mut running = JoinSet::new();
    running.spawn(async move {
        caller_router
            .decide_with(Some(&messages), || Some(caller()))
            .await
    });
    tokio::task::yield_now().await;
    assert_eq!(tasks.len(), 1, "the build runs on the caller's tracker");
    let finished = running.join_next().await.expect("the call finishes");
    assert_eq!(finished.unwrap().unwrap(), decisions(Some("code"), "code"));
    tasks.close();
    tasks.wait().await;
    assert!(tasks.is_empty(), "the tracker drains once the build ends");
}

#[tokio::test]
async fn a_missing_caller_falls_back_without_a_query_embedding() {
    let embedder = Arc::new(FakeEmbedder::new(&embedding_arena()));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    let decision = router.decide_with(Some(&messages), || None).await.unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
    assert_eq!(embedder.document_calls().len(), 2);
    assert!(embedder.query_calls().is_empty());
}

#[tokio::test]
async fn a_failed_build_is_reported_before_the_caller_is_read() {
    let embedder = Arc::new(FakeEmbedder::new(&[]).failing_documents(1));
    let router = router(TWO_ROUTES, &embedder);
    let messages = vec![user_text("fix it")];
    let mut read = false;
    let result = router
        .decide_with(Some(&messages), || {
            read = true;
            None
        })
        .await;
    assert!(result.is_err());
    assert!(!read, "the caller is read only once the index exists");
}

#[rstest]
#[case(vec![user_text("second")], "second")]
#[case(vec![user_text("first"), user_text("last")], "last")]
#[case(vec![user_text("first"), message(Role::Assistant, None)], "first")]
#[case(vec![message(Role::User, None)], "")]
#[case(vec![message(Role::System, None), message(Role::Assistant, None)], "")]
#[case(vec![message(Role::Tool, None), message(Role::Assistant, None)], "")]
#[case(vec![], "")]
#[case(
    vec![user_parts(vec![text_part("hello"), image_part(), text_part("world")])],
    "hello world"
)]
#[case(vec![user_parts(vec![image_part()])], "")]
#[case(
    vec![user_parts(vec![
        text_part("a"),
        keyless_text_part(),
        numeric_text_part(),
        text_part("b")
    ])],
    "a b"
)]
#[tokio::test]
async fn the_query_carries_the_last_user_text(
    #[case] messages: Vec<ChatMessage>,
    #[case] expected: &str,
) {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        (expected, vec![1.0, 0.0]),
    ]));
    let router = router(r#"{"routes": []}"#, &embedder);
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
    assert_eq!(embedder.query_calls()[0].text, expected);
}

#[tokio::test]
async fn the_query_is_clamped_by_characters_and_reported_whole() {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("te", vec![1.0, 0.0]),
        ("aé", vec![1.0, 0.0]),
    ]));
    let configured = with_limit(settings(r#"{"routes": []}"#), 2);
    let router = AutoRouter::new(configured, Arc::clone(&embedder), TaskTracker::new())
        .expect("the config parses");
    let messages = vec![user_text("aé中🙂")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
    let query = &embedder.query_calls()[0];
    assert_eq!(query.text, "aé");
    assert_eq!(query.raw_text, "aé中🙂");
    assert_eq!(embedder.document_calls()[0].texts, vec!["te"]);
}

#[tokio::test]
async fn a_caller_context_reaches_the_query_embedding() {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("fix it", vec![1.0, 0.0]),
    ]));
    let router = router(r#"{"routes": []}"#, &embedder);
    let context = CallerContext {
        metadata: IndexMap::from([(
            "user_api_key_team_id".to_owned(),
            Json::Text("team-1".to_owned()),
        )]),
        litellm_metadata: IndexMap::from([(
            "user_api_key".to_owned(),
            Json::Text("key-1".to_owned()),
        )]),
        turn_off_message_logging: Some(true),
        litellm_session_id: Some("session-1".to_owned()),
        litellm_trace_id: Some("trace-1".to_owned()),
    };
    let messages = vec![user_text("fix it")];
    router
        .decide_with(Some(&messages), || Some(context.clone()))
        .await
        .unwrap();
    let query = &embedder.query_calls()[0];
    assert_eq!(query.caller, context);
    assert_eq!(query.raw_text, "fix it");
}

#[tokio::test]
async fn a_configured_path_wins_over_the_inline_config() {
    let directory = temporary_directory("path-wins");
    let path = directory.join("routes.json");
    fs::write(
        &path,
        r#"{"encoder_type": "openai", "encoder_name": "e", "routes": [{"name": "from_path", "utterances": ["p"]}]}"#,
    )
    .unwrap();
    let inline = r#"{"routes": [{"name": "from_inline", "utterances": ["i"]}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("p", vec![1.0, 0.0]),
        ("i", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let configured = AutoRouterSettings {
        config_path: Some(path),
        config_json: Some(inline.to_owned()),
        default_model: "fallback".to_owned(),
        embedding_model: "fake-embed".to_owned(),
        max_input_chars: None,
    };
    let router = AutoRouter::new(configured, Arc::clone(&embedder), TaskTracker::new())
        .expect("the file parses");
    let messages = vec![user_text("ask")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(Some("from_path"), "from_path"));
    assert_eq!(embedder.utterance_texts(), vec!["p"]);
    fs::remove_dir_all(&directory).unwrap();
}

#[tokio::test]
async fn an_empty_path_falls_through_to_the_inline_config() {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("i", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let configured = AutoRouterSettings {
        config_path: Some(PathBuf::from("")),
        config_json: Some(r#"{"routes": [{"name": "inline", "utterances": ["i"]}]}"#.to_owned()),
        default_model: "fallback".to_owned(),
        embedding_model: "fake-embed".to_owned(),
        max_input_chars: None,
    };
    let router = AutoRouter::new(configured, Arc::clone(&embedder), TaskTracker::new())
        .expect("the inline config parses");
    let messages = vec![user_text("ask")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(Some("inline"), "inline"));
}

#[tokio::test]
async fn a_route_without_utterances_never_matches() {
    let config = r#"{"routes": [{"name": "empty", "utterances": []}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("ask")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
    assert_eq!(embedder.document_calls().len(), 1);
}

#[tokio::test]
async fn an_empty_route_name_keeps_the_route_but_uses_the_default_model() {
    let config = r#"{"routes": [{"name": "", "utterances": ["ask"]}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("ask")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(Some(""), "fallback"));
}

#[tokio::test]
async fn duplicate_utterances_are_embedded_once() {
    let config = r#"{"routes": [{"name": "r", "utterances": ["same", "same"]}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("same", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("ask")];
    router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(embedder.utterance_texts(), vec!["same"]);
}

#[tokio::test]
async fn utterances_reach_the_embedder_in_the_sync_order() {
    let config = r#"{"routes": [
        {"name": "r1", "utterances": ["a b"]},
        {"name": "r10", "utterances": ["a"]},
        {"name": "r2", "utterances": ["a b", "a"]}
    ]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("a", vec![1.0, 0.0]),
        ("a b", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("ask")];
    router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(
        embedder.utterance_texts(),
        vec!["a", "a b", "a b", "a"],
        "the composed sync key orders utterances, not the two fields"
    );
}

#[tokio::test]
async fn a_repeated_route_name_keeps_the_first_threshold() {
    let config = r#"{"routes": [
        {"name": "r", "utterances": ["high"], "score_threshold": 0.6},
        {"name": "r", "utterances": ["low"], "score_threshold": 0.1}
    ]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("high", vec![1.0, 0.0]),
        ("low", vec![0.0, 1.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("ask")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(
        decision,
        decisions(None, "fallback"),
        "the mean 0.5 stays below the first route's 0.6 threshold"
    );
}

#[tokio::test]
async fn a_route_uses_the_default_threshold_when_it_sets_none() {
    let config = r#"{"routes": [{"name": "close", "utterances": ["close"]}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("close", vec![1.0, 1.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let messages = vec![user_text("ask")];
    let decision = router
        .decide_with(Some(&messages), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(Some("close"), "close"));
}

#[tokio::test]
async fn no_messages_means_no_routing_decision() {
    let embedder = Arc::new(FakeEmbedder::new(&[]));
    let router = router(TWO_ROUTES, &embedder);
    assert_eq!(
        router.decide_with(None, || Some(caller())).await.unwrap(),
        None
    );
    assert_eq!(router.decide_with(None, || None).await.unwrap(), None);
    assert!(embedder.document_calls().is_empty());
    assert!(embedder.query_calls().is_empty());
}

#[test]
fn a_config_without_any_source_is_missing_config() {
    let embedder = Arc::new(FakeEmbedder::new(&[]));
    let configured = AutoRouterSettings {
        config_path: None,
        config_json: None,
        default_model: "fallback".to_owned(),
        embedding_model: "fake-embed".to_owned(),
        max_input_chars: None,
    };
    let Err(error) = AutoRouter::new(configured, embedder, TaskTracker::new()) else {
        panic!("there is no config to load");
    };
    assert!(matches!(error, RouteIndexError::MissingConfig));
}

#[test]
fn an_empty_inline_config_is_missing_config_too() {
    let embedder = Arc::new(FakeEmbedder::new(&[]));
    let configured = AutoRouterSettings {
        config_path: Some(PathBuf::new()),
        config_json: Some(String::new()),
        default_model: "fallback".to_owned(),
        embedding_model: "fake-embed".to_owned(),
        max_input_chars: None,
    };
    let Err(error) = AutoRouter::new(configured, embedder, TaskTracker::new()) else {
        panic!("there is no config to load");
    };
    assert!(matches!(error, RouteIndexError::MissingConfig));
}
