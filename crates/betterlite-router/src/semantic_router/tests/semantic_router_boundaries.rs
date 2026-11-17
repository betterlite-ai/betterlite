use std::fs;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;

use crate::semantic_math::MathError;
use crate::semantic_router::build::{BuildState, wait_for_build};
use crate::semantic_router::error::{EmbeddingStage, RouterError};
use crate::semantic_router::index::{
    ROUTE_INDEX_MAX_BYTES, RouteIndexError, load_file, parse_inline,
};
use crate::semantic_router::select::BuiltEntry;
use crate::semantic_router::service::AutoRouter;
use crate::semantic_router::tests::support::{
    FakeEmbedder, FakeError, TWO_ROUTES, caller, decisions, router, settings, temporary_directory,
    user_text, with_limit,
};

// the smallest config the parser accepts, padded to an exact byte length with trailing whitespace
fn padded_config(len: usize) -> String {
    let skeleton = r#"{"routes":[]}"#;
    assert!(
        len >= skeleton.len(),
        "the skeleton is the smallest padded config"
    );
    format!(
        "{}{}",
        skeleton,
        " ".repeat(len.saturating_sub(skeleton.len()))
    )
}

#[test]
fn an_inline_config_at_the_byte_limit_parses_and_one_byte_more_is_rejected() {
    let at_limit = padded_config(ROUTE_INDEX_MAX_BYTES);
    assert_eq!(at_limit.len(), ROUTE_INDEX_MAX_BYTES);
    assert!(parse_inline(&at_limit).is_ok());
    let over = padded_config(ROUTE_INDEX_MAX_BYTES + 1);
    let Err(error) = parse_inline(&over) else {
        panic!("a config one byte past the bound is rejected");
    };
    assert!(matches!(
        &error,
        RouteIndexError::TooLarge { limit } if *limit == ROUTE_INDEX_MAX_BYTES
    ));
    assert_eq!(error.code(), "too_large");
}

#[test]
fn a_config_file_at_the_byte_limit_parses_and_one_byte_more_is_rejected() {
    let directory = temporary_directory("file-bound");
    let at_limit = directory.join("at.json");
    fs::write(&at_limit, padded_config(ROUTE_INDEX_MAX_BYTES)).unwrap();
    assert!(load_file(&at_limit).is_ok());
    let over = directory.join("over.json");
    fs::write(&over, padded_config(ROUTE_INDEX_MAX_BYTES + 1)).unwrap();
    let Err(error) = load_file(&over) else {
        panic!("a file one byte past the bound is rejected");
    };
    assert!(matches!(&error, RouteIndexError::TooLarge { .. }));
    assert_eq!(error.code(), "too_large");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_config_file_with_an_unknown_extension_is_rejected() {
    let directory = temporary_directory("extension");
    let path = directory.join("routes.txt");
    fs::write(&path, r#"{"routes":[]}"#).unwrap();
    let Err(error) = load_file(&path) else {
        panic!("a .txt file is not a route config");
    };
    assert!(
        matches!(&error, RouteIndexError::UnsupportedExtension { path: rejected } if rejected == &path)
    );
    assert_eq!(error.code(), "unsupported_extension");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_missing_config_file_is_a_read_error() {
    let directory = temporary_directory("missing");
    let path = directory.join("absent.json");
    let Err(error) = load_file(&path) else {
        panic!("a file that does not exist cannot be read");
    };
    assert!(matches!(&error, RouteIndexError::Read { path: rejected, .. } if rejected == &path));
    assert_eq!(error.code(), "read_failed");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_nonfinite_threshold_is_rejected() {
    let directory = temporary_directory("nonfinite");
    let path = directory.join("routes.yaml");
    fs::write(
        &path,
        "encoder_type: openai\nencoder_name: text-embedding-3-small\nroutes:\n  - name: r\n    utterances: [a]\n    score_threshold: .nan\n",
    )
    .unwrap();
    let Err(error) = load_file(&path) else {
        panic!("a NaN threshold is not a usable route");
    };
    assert!(matches!(&error, RouteIndexError::NonFiniteThreshold { name } if name == "r"));
    assert_eq!(error.code(), "non_finite_threshold");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_file_route_without_utterances_is_rejected() {
    let directory = temporary_directory("utterances");
    let path = directory.join("routes.yaml");
    fs::write(
        &path,
        "encoder_type: openai\nencoder_name: e\nroutes:\n  - name: r\n",
    )
    .unwrap();
    let Err(error) = load_file(&path) else {
        panic!("the file format requires utterances on every route");
    };
    assert!(matches!(error, RouteIndexError::Yaml(_)));
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_file_without_the_encoder_keys_is_rejected() {
    let directory = temporary_directory("no-encoder");
    let path = directory.join("routes.json");
    fs::write(&path, r#"{"routes": []}"#).unwrap();
    let Err(error) = load_file(&path) else {
        panic!("the file format requires the encoder keys");
    };
    assert!(matches!(error, RouteIndexError::Json(_)));
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_file_route_keeps_the_fields_the_encoder_ignores() {
    let directory = temporary_directory("extra-fields");
    let path = directory.join("routes.json");
    fs::write(
        &path,
        r#"{
            "encoder_type": "openai",
            "encoder_name": "text-embedding-3-small",
            "routes": [{
                "name": "r",
                "description": "d",
                "utterances": ["a"],
                "score_threshold": 0.4,
                "function_schemas": [{"name": "f"}],
                "metadata": {"team": "t"},
                "llm": {"module": "m", "class": "C", "model": "gpt"}
            }]
        }"#,
    )
    .unwrap();
    let index = load_file(&path).expect("the extra route fields are accepted");
    assert_eq!(index.rows.len(), 1);
    assert_eq!(index.rows[0].route, "r");
    assert_eq!(index.rows[0].text, "a");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_config_without_a_routes_key_is_an_empty_index() {
    let index = parse_inline(r#"{"encoder_type": "openai"}"#).expect("the config parses");
    assert!(index.rows.is_empty());
    assert!(index.groups.is_empty());
}

#[rstest]
#[case::misspelled_utterances(r#"{"routes": [{"name": "r", "utterance": ["a"]}]}"#, "utterance")]
#[case::misspelled_threshold(
    r#"{"routes": [{"name": "r", "utterances": ["a"], "score_treshold": 0.4}]}"#,
    "score_treshold"
)]
#[case::misspelled_top_level(r#"{"routse": []}"#, "routse")]
fn a_misspelled_inline_config_key_is_rejected(#[case] source: &str, #[case] key: &str) {
    let Err(error) = parse_inline(source) else {
        panic!("{key} is not a key of the route config");
    };
    assert!(matches!(error, RouteIndexError::Json(_)));
    assert!(
        error
            .to_string()
            .contains(&format!("unknown field `{key}`")),
        "{error}"
    );
}

#[rstest]
#[case::misspelled_utterances(
    r#"{"encoder_type": "openai", "encoder_name": "e", "routes": [{"name": "r", "utterance": ["a"]}]}"#,
    "utterance"
)]
#[case::misspelled_threshold(
    r#"{"encoder_type": "openai", "encoder_name": "e", "routes": [{"name": "r", "utterances": ["a"], "score_treshold": 0.4}]}"#,
    "score_treshold"
)]
#[case::misspelled_top_level(
    r#"{"encoder_type": "openai", "encoder_name": "e", "routse": []}"#,
    "routse"
)]
fn a_misspelled_file_config_key_is_rejected(#[case] source: &str, #[case] key: &str) {
    let directory = temporary_directory("misspelled");
    let path = directory.join("routes.json");
    fs::write(&path, source).unwrap();
    let Err(error) = load_file(&path) else {
        panic!("{key} is not a key of the route config file");
    };
    assert!(matches!(error, RouteIndexError::Json(_)));
    assert!(
        error
            .to_string()
            .contains(&format!("unknown field `{key}`")),
        "{error}"
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn an_inline_route_keeps_the_fields_the_encoder_ignores() {
    let index = parse_inline(
        r#"{"routes": [{"name": "r", "utterances": ["a"], "function_schemas": [{"name": "f"}], "metadata": {"team": "t"}, "llm": {"module": "m"}}]}"#,
    )
    .expect("the dynamic route fields are accepted");
    assert_eq!(index.rows.len(), 1);
    assert_eq!(index.rows[0].route, "r");
}

#[tokio::test]
async fn a_threshold_of_one_matches_only_an_identical_vector() {
    let config =
        r#"{"routes": [{"name": "exact", "utterances": ["same"], "score_threshold": 1.0}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("same", vec![1.0, 0.0]),
        ("exact", vec![1.0, 0.0]),
        ("near", vec![1.0, 0.000001]),
    ]));
    let router = router(config, &embedder);
    let matched = router
        .decide_with(Some(&[user_text("exact")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(matched, decisions(Some("exact"), "exact"));
    let missed = router
        .decide_with(Some(&[user_text("near")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(missed, decisions(None, "fallback"));
    assert_eq!(embedder.document_calls().len(), 2);
}

#[tokio::test]
async fn a_zero_threshold_matches_even_a_negative_score() {
    let config =
        r#"{"routes": [{"name": "zero", "utterances": ["same"], "score_threshold": 0.0}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("same", vec![0.0, 1.0]),
        ("opposite", vec![0.0, -1.0]),
    ]));
    let router = router(config, &embedder);
    let decision = router
        .decide_with(Some(&[user_text("opposite")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(Some("zero"), "zero"));
}

#[tokio::test]
async fn a_negative_threshold_still_compares() {
    let config =
        r#"{"routes": [{"name": "negative", "utterances": ["same"], "score_threshold": -0.5}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("same", vec![0.0, 1.0]),
        ("opposite", vec![0.0, -1.0]),
        ("close", vec![1.0, -0.2]),
    ]));
    let router = router(config, &embedder);
    let missed = router
        .decide_with(Some(&[user_text("opposite")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(missed, decisions(None, "fallback"));
    let matched = router
        .decide_with(Some(&[user_text("close")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(matched, decisions(Some("negative"), "negative"));
}

#[tokio::test]
async fn a_route_mean_counts_the_rows_inside_the_top_five() {
    let config = r#"{"routes": [
        {"name": "a", "utterances": ["u1", "u2", "u3", "u4", "u5"], "score_threshold": 0.98},
        {"name": "b", "utterances": ["v1"], "score_threshold": 0.9}
    ]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
        ("u1", vec![0.99, 0.1410673597966589]),
        ("u2", vec![0.98, 0.1989974874213241]),
        ("u3", vec![0.97, 0.2431049156228645]),
        ("u4", vec![0.96, 0.28000000000000014]),
        ("u5", vec![0.19999999999999993, 0.9797958971132713]),
        ("v1", vec![0.93, 0.367559518989782]),
    ]));
    let router = router(config, &embedder);
    let decision = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(
        decision,
        decisions(Some("b"), "b"),
        "the fifth row's 0.98 threshold keeps route a out of the window"
    );
}

#[tokio::test]
async fn a_route_mean_ignores_the_rows_past_the_top_five() {
    let config = r#"{"routes": [
        {"name": "a", "utterances": ["w1", "w2", "w3", "w4", "w5"], "score_threshold": 0.9},
        {"name": "b", "utterances": ["x1", "x2"], "score_threshold": 0.99}
    ]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
        ("w1", vec![0.97, 0.2431049156228645]),
        ("w2", vec![0.97, 0.2431049156228645]),
        ("w3", vec![0.97, 0.2431049156228645]),
        ("w4", vec![0.97, 0.2431049156228645]),
        ("w5", vec![0.1, 0.99498743710662]),
        ("x1", vec![0.995, 0.09987492177719094]),
        ("x2", vec![0.94, 0.34117444218463977]),
    ]));
    let router = router(config, &embedder);
    let decision = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(
        decision,
        decisions(Some("b"), "b"),
        "route b's second row sits past the top five and never lowers its mean"
    );
}

#[tokio::test]
async fn an_utterance_vector_is_scored_by_cosine() {
    let config =
        r#"{"routes": [{"name": "tilted", "utterances": ["tilted"], "score_threshold": 0.9}]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("tilted", vec![1.0, 1.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let decision = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(
        decision,
        decisions(None, "fallback"),
        "the tilted utterance scores 1/sqrt(2), below the 0.9 threshold"
    );
}

#[tokio::test]
async fn tied_routes_keep_the_earlier_route() {
    let config = r#"{"routes": [
        {"name": "first", "utterances": ["same"]},
        {"name": "second", "utterances": ["same2"]}
    ]}"#;
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("same", vec![1.0, 0.0]),
        ("same2", vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(config, &embedder);
    let decision = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(Some("first"), "first"));
}

#[tokio::test]
async fn a_probe_batch_of_the_wrong_size_is_a_vector_count_error() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[]).documents_answer(&["test"], vec![vec![1.0, 0.0], vec![0.0, 1.0]]),
    );
    let router = router(TWO_ROUTES, &embedder);
    let error = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .expect_err("a two-vector probe cannot size the index");
    assert!(matches!(
        error.as_ref(),
        RouterError::VectorCount {
            stage: EmbeddingStage::Documents,
            expected: 1,
            found: 2
        }
    ));
}

#[tokio::test]
async fn a_probe_batch_without_vectors_is_a_vector_count_error() {
    let embedder = Arc::new(FakeEmbedder::new(&[]).documents_answer(&["test"], Vec::new()));
    let router = router(TWO_ROUTES, &embedder);
    let error = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .expect_err("an empty probe cannot size the index");
    assert!(matches!(
        error.as_ref(),
        RouterError::VectorCount {
            stage: EmbeddingStage::Documents,
            expected: 1,
            found: 0
        }
    ));
}

#[tokio::test]
async fn an_utterance_batch_of_the_wrong_size_is_a_vector_count_error() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[("test", vec![1.0, 0.0])])
            .documents_answer(&["small talk", "write code"], vec![vec![1.0, 0.0]]),
    );
    let router = router(TWO_ROUTES, &embedder);
    let error = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .expect_err("one vector cannot describe two utterances");
    assert!(matches!(
        error.as_ref(),
        RouterError::VectorCount {
            stage: EmbeddingStage::Documents,
            expected: 2,
            found: 1
        }
    ));
}

#[tokio::test]
async fn a_ragged_document_batch_is_rejected() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[("test", vec![1.0, 0.0])]).documents_answer(
            &["small talk", "write code"],
            vec![vec![1.0, 0.0], vec![1.0]],
        ),
    );
    let router = router(TWO_ROUTES, &embedder);
    let error = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .expect_err("a ragged batch breaks the vector width contract");
    assert!(matches!(
        error.as_ref(),
        RouterError::Vectors {
            stage: EmbeddingStage::Documents,
            source: MathError::Dimension {
                index: 1,
                expected: 2,
                found: 1
            }
        }
    ));
}

#[tokio::test]
async fn a_nonfinite_document_vector_is_rejected() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[("test", vec![1.0, 0.0])]).documents_answer(
            &["small talk", "write code"],
            vec![vec![1.0, 0.0], vec![f64::NAN, 0.0]],
        ),
    );
    let router = router(TWO_ROUTES, &embedder);
    let error = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .expect_err("a NaN document vector cannot be scored");
    assert!(matches!(
        error.as_ref(),
        RouterError::Vectors {
            stage: EmbeddingStage::Documents,
            source: MathError::NonFinite { index: 1 }
        }
    ));
}

#[tokio::test]
async fn a_zero_norm_document_vector_is_rejected() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[("test", vec![1.0, 0.0])]).documents_answer(
            &["small talk", "write code"],
            vec![vec![1.0, 0.0], vec![0.0, 0.0]],
        ),
    );
    let router = router(TWO_ROUTES, &embedder);
    let error = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .expect_err("a zero document vector has no cosine");
    assert!(matches!(
        error.as_ref(),
        RouterError::Vectors {
            stage: EmbeddingStage::Documents,
            source: MathError::ZeroNorm { index: 1 }
        }
    ));
}

#[rstest]
#[case::one_dimension(vec![vec![1.0]])]
#[case::no_vector(Vec::<Vec<f64>>::new())]
#[case::infinite(vec![vec![f64::INFINITY, 0.0]])]
#[case::zero_norm(vec![vec![0.0, 0.0]])]
#[tokio::test]
async fn a_bad_query_vector_falls_back_to_the_default_model(#[case] vectors: Vec<Vec<f64>>) {
    let embedder = Arc::new(
        FakeEmbedder::new(&[
            ("test", vec![1.0, 0.0]),
            ("write code", vec![1.0, 0.0]),
            ("small talk", vec![0.0, 1.0]),
        ])
        .query_answers("ask", vectors),
    );
    let router = router(TWO_ROUTES, &embedder);
    let decision = router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(decision, decisions(None, "fallback"));
    assert_eq!(embedder.query_calls().len(), 1);
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
#[tokio::test]
async fn a_limit_of_zero_or_less_embeds_the_whole_text(#[case] limit: i64) {
    let text = "c".repeat(10);
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        (text.as_str(), vec![1.0, 0.0]),
    ]));
    let configured = with_limit(settings(r#"{"routes": []}"#), limit);
    let router = AutoRouter::new(configured, Arc::clone(&embedder), TaskTracker::new())
        .expect("the config parses");
    router
        .decide_with(Some(&[user_text(&text)]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(embedder.query_calls()[0].text, text);
}

#[tokio::test]
async fn the_default_limit_is_two_thousand_characters() {
    let raw = "a".repeat(2001);
    let clamped = "a".repeat(2000);
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        (clamped.as_str(), vec![1.0, 0.0]),
    ]));
    let router = router(r#"{"routes": []}"#, &embedder);
    router
        .decide_with(Some(&[user_text(&raw)]), || Some(caller()))
        .await
        .unwrap();
    let query = &embedder.query_calls()[0];
    assert_eq!(query.text, clamped);
    assert_eq!(query.text.len(), 2000);
    assert_eq!(query.raw_text, raw);
}

#[tokio::test]
async fn the_character_limit_applies_to_each_utterance() {
    let raw = "b".repeat(2001);
    let clamped = "b".repeat(2000);
    let config = serde_json::json!({"routes": [{"name": "r", "utterances": [raw]}]}).to_string();
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        (clamped.as_str(), vec![1.0, 0.0]),
        ("ask", vec![1.0, 0.0]),
    ]));
    let router = router(&config, &embedder);
    router
        .decide_with(Some(&[user_text("ask")]), || Some(caller()))
        .await
        .unwrap();
    assert_eq!(embedder.utterance_texts(), vec![clamped]);
}

#[tokio::test]
async fn a_dropped_build_sender_aborts_the_waiter() {
    let (sender, receiver) = watch::channel(BuildState::<FakeError>::Building);
    drop(sender);
    let error = wait_for_build(receiver)
        .await
        .expect_err("the build never completed");
    assert!(matches!(error.as_ref(), RouterError::BuildAborted));
}

#[tokio::test]
async fn a_failed_build_reaches_the_waiter() {
    let (sender, receiver) = watch::channel(BuildState::<FakeError>::Building);
    sender.send_replace(BuildState::Done(Err(Arc::new(RouterError::BuildAborted))));
    let error = wait_for_build(receiver)
        .await
        .expect_err("the build failed");
    assert!(matches!(error.as_ref(), RouterError::BuildAborted));
}

#[tokio::test]
async fn a_done_build_reaches_the_waiter() {
    let entries = Arc::new(vec![BuiltEntry {
        route: "r".to_owned(),
        threshold: 0.5,
        vector: vec![1.0, 0.0],
    }]);
    let (sender, receiver) = watch::channel(BuildState::<FakeError>::Building);
    sender.send_replace(BuildState::Done(Ok(Arc::clone(&entries))));
    let received = wait_for_build(receiver).await.expect("the build completed");
    assert_eq!(received.as_ref(), entries.as_ref());
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_waiter_does_not_cancel_the_shared_build() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[
            ("test", vec![1.0, 0.0]),
            ("write code", vec![1.0, 0.0]),
            ("small talk", vec![0.0, 1.0]),
            ("fix it", vec![1.0, 0.0]),
        ])
        .with_delay(Duration::from_millis(10)),
    );
    let router = Arc::new(router(TWO_ROUTES, &embedder));
    let messages = vec![user_text("fix it")];
    let mut tasks = JoinSet::new();
    let cancelled_router = Arc::clone(&router);
    let cancelled_messages = messages.clone();
    let cancelled = tasks.spawn(async move {
        cancelled_router
            .decide_with(Some(&cancelled_messages), || Some(caller()))
            .await
    });
    tokio::task::yield_now().await;
    cancelled.abort();
    let survivor_router = Arc::clone(&router);
    let survivor_messages = messages.clone();
    tasks.spawn(async move {
        survivor_router
            .decide_with(Some(&survivor_messages), || Some(caller()))
            .await
    });
    let first = tasks.join_next().await.expect("a task finishes");
    assert!(first.unwrap_err().is_cancelled());
    let second = tasks.join_next().await.expect("the survivor finishes");
    assert_eq!(second.unwrap().unwrap(), decisions(Some("code"), "code"));
    assert_eq!(embedder.document_calls().len(), 2);
    assert_eq!(embedder.query_calls().len(), 1);
}
