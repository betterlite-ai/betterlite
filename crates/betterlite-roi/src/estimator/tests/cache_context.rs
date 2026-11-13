#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use crate::estimator::cache::cache_context;
use crate::estimator::json::json_dumps_text;
use crate::estimator::models::EstimatorModel;

use super::support::{ModelSupport, settings};

const PROMPT: &str = "Estimate this pull request.";

#[tokio::test]
async fn cache_context_hashes_the_litellm_configuration() {
    let settings = settings("gpt-5.2", PROMPT);
    let mut predicates = ModelSupport {
        supported: Vec::new(),
        seen: Vec::new(),
    };
    let digest = cache_context(&settings, Some(&[]), &mut predicates)
        .await
        .unwrap();
    assert_eq!(
        digest,
        "7628fc7ae721004ec205c05a27a670a332aff4e4d993a6fa475f969446e8c8ee"
    );
    assert!(predicates.seen.is_empty());
}

#[tokio::test]
async fn cache_context_hashes_reasoning_effort_when_every_model_supports_it() {
    let settings = settings("gpt-5.2", PROMPT);
    let models: Vec<EstimatorModel> = vec![("gpt-5.2".to_owned(), None)];
    let mut predicates = ModelSupport {
        supported: vec!["gpt-5.2".to_owned()],
        seen: Vec::new(),
    };
    let digest = cache_context(&settings, Some(&models), &mut predicates)
        .await
        .unwrap();
    assert_eq!(
        digest,
        "4a51473469234221eb22977570f4a26909b5a4af6a16d120381c76c180e87172"
    );
}

#[tokio::test]
async fn cache_context_is_stable_and_independent_of_pull_evidence() {
    let settings = settings("gpt-5.2", PROMPT);
    let mut first = ModelSupport {
        supported: Vec::new(),
        seen: Vec::new(),
    };
    let mut second = ModelSupport {
        supported: Vec::new(),
        seen: Vec::new(),
    };
    let first_digest = cache_context(&settings, Some(&[]), &mut first)
        .await
        .unwrap();
    let second_digest = cache_context(&settings, Some(&[]), &mut second)
        .await
        .unwrap();
    assert_eq!(first_digest, second_digest);
}

#[tokio::test]
async fn cache_context_changes_when_the_configuration_changes() {
    let settings = settings("deepseek-chat", "Estimate this.");
    let mut predicates = ModelSupport {
        supported: Vec::new(),
        seen: Vec::new(),
    };
    let digest = cache_context(&settings, Some(&[]), &mut predicates)
        .await
        .unwrap();
    assert_eq!(
        digest,
        "e1a892b5baf87f3d805454ec589f36d482906f9fa786cd30a28a24fa58d1440e"
    );
}

#[rstest]
#[case::array(json!([1, "x", null]), r#"[1, "x", null]"#)]
#[case::object(json!({"a": true, "b": {}}), r#"{"a": true, "b": {}}"#)]
#[case::unicode(json!({"title": "café"}), "{\"title\": \"café\"}")]
fn json_dumps_text_matches_json_dumps_default_separators(
    #[case] value: serde_json::Value,
    #[case] expected: &str,
) {
    assert_eq!(json_dumps_text(&value).unwrap(), expected);
}
