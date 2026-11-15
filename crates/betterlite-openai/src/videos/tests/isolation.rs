use serde_json::Value;

use crate::videos::config::{
    DEFAULT_API_BASE, VideoCredentials, complete_url, validate_environment,
};
use crate::videos::error::{VideoError, get_error_class};

use super::support::{credentials, headers};

#[test]
fn openai_videos_credential_identity_isolation() {
    let first = credentials(Some("sk-first"), None, None, None, None);
    let second = credentials(None, None, Some("sk-second"), None, None);
    let base = headers(&[("Authorization", "Bearer old")]);
    let first_headers = validate_environment(base.clone(), &first);
    let second_headers = validate_environment(base.clone(), &second);
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-first")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-second")
    );
    assert_eq!(
        base.get("Authorization").map(String::as_str),
        Some("Bearer old")
    );
    assert_ne!(first_headers, second_headers);

    let environment = credentials(None, None, None, None, Some("sk-environment"));
    assert_eq!(
        validate_environment(base.clone(), &environment)
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-environment")
    );
    assert_eq!(
        validate_environment(base, &first)
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-first")
    );

    let unconfigured = VideoCredentials::default();
    assert_eq!(
        validate_environment(headers(&[]), &unconfigured)
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer None")
    );
    assert_eq!(complete_url(None), format!("{DEFAULT_API_BASE}/videos"));
}

#[test]
fn openai_videos_secret_safe_failure() {
    let secret = "sk-live-fixture-secret";
    let configured = credentials(
        Some(secret),
        Some(secret),
        Some(secret),
        Some(secret),
        Some(secret),
    );
    let debugged = format!("{configured:?}");
    assert!(!debugged.contains(secret));
    assert_eq!(debugged.matches("[REDACTED]").count(), 5);

    let failures = [
        VideoError::MissingVideoId,
        VideoError::DotSegment,
        VideoError::ExpectedVideo,
        VideoError::Reply {
            source: serde_json::from_str::<Value>("{").unwrap_err(),
        },
    ];
    for failure in failures {
        assert!(!format!("{failure} {failure:?}").contains(secret));
    }

    let error = get_error_class("rate limited", 429, headers(&[("retry-after", "1")]));
    assert!(!format!("{error:?}").contains(secret));
    assert!(!serde_json::to_string(&error).unwrap().contains(secret));
}
