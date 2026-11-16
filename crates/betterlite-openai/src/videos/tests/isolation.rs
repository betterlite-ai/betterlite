use betterlite_http::error::HttpError;
use serde_json::Value;

use crate::videos::config::{
    DEFAULT_API_BASE, VideoCredentials, complete_url, validate_environment,
};
use crate::videos::error::{VideoError, get_error_class};

use super::support::{credentials, exposed, headers, secret_headers};

#[test]
fn openai_videos_credential_identity_isolation() {
    let first = credentials(Some("sk-first"), None, None, None, None);
    let second = credentials(None, None, Some("sk-second"), None, None);
    let base = secret_headers(&[("Authorization", "Bearer old")]);
    let first_headers = validate_environment(base.clone(), &first);
    let second_headers = validate_environment(base.clone(), &second);
    assert_eq!(
        exposed(&first_headers, "Authorization"),
        Some("Bearer sk-first")
    );
    assert_eq!(
        exposed(&second_headers, "Authorization"),
        Some("Bearer sk-second")
    );
    assert_eq!(exposed(&base, "Authorization"), Some("Bearer old"));

    let environment = credentials(None, None, None, None, Some("sk-environment"));
    assert_eq!(
        exposed(
            &validate_environment(base.clone(), &environment),
            "Authorization"
        ),
        Some("Bearer sk-environment")
    );
    assert_eq!(
        exposed(&validate_environment(base, &first), "Authorization"),
        Some("Bearer sk-first")
    );

    let unconfigured = VideoCredentials::default();
    assert_eq!(
        exposed(
            &validate_environment(secret_headers(&[]), &unconfigured),
            "Authorization"
        ),
        Some("Bearer None")
    );
    assert_eq!(complete_url(None), format!("{DEFAULT_API_BASE}/videos"));
}

fn failure_samples() -> [VideoError; 3] {
    [
        VideoError::Path {
            source: HttpError::Required {
                field_name: "video_id".to_owned(),
            },
        },
        VideoError::ExpectedVideo,
        VideoError::Reply {
            source: serde_json::from_str::<Value>("{").unwrap_err(),
        },
    ]
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

    let returned = validate_environment(secret_headers(&[("X-Trace", "trace-1")]), &configured);
    assert_eq!(
        exposed(&returned, "Authorization"),
        Some("Bearer sk-live-fixture-secret")
    );
    assert!(!format!("{returned:?}").contains(secret));

    let failures = failure_samples();
    assert!(
        failures
            .iter()
            .all(|failure| !format!("{failure} {failure:?}").contains(secret))
    );

    let error = get_error_class("rate limited", 429, headers(&[("retry-after", "1")]));
    assert!(!format!("{error:?}").contains(secret));
    assert!(!serde_json::to_string(&error).unwrap().contains(secret));
}
