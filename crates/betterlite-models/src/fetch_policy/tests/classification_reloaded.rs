use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, FetchedCatalog, classify_fetch_response};
use crate::fetch_policy::revision::git_blob_id;

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let body = r#"{"model":{"litellm_provider":"openai"},"sample_spec":{}}"#;
    let response = support::response_with_headers(200, &[("ETag", "abc123")], body);
    let fetched = reloaded(classify_fetch_response(response, support::URL));
    assert_eq!(fetched.revision, git_blob_id(body.as_bytes()));
    assert_eq!(fetched.etag.as_deref(), Some("abc123"));
    assert_eq!(fetched.body.as_slice(), body.as_bytes());
    assert_eq!(
        fetched
            .models
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["model", "sample_spec"]
    );

    let created = reloaded(classify_fetch_response(
        support::response(201, "{}"),
        support::URL,
    ));
    assert_eq!(created.etag, None);
    assert_eq!(created.revision, git_blob_id(b"{}"));

    let repeated = support::response_with_headers(200, &[("etag", "a"), ("ETag", "b")], "{}");
    let joined = reloaded(classify_fetch_response(repeated, support::URL));
    assert_eq!(joined.etag.as_deref(), Some("a, b"));
}

fn reloaded(outcome: AttemptOutcome) -> FetchedCatalog {
    match outcome {
        AttemptOutcome::Reloaded(fetched) => fetched,
        AttemptOutcome::Unavailable(_) | AttemptOutcome::Retryable(_) => {
            panic!("expected reloaded")
        }
    }
}
