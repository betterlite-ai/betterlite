use std::error::Error;
use std::io;

use betterlite_models::decisions::Json;

use super::support::{self, SECRET};
use crate::realtime_events::failure::{
    ProviderFailure, ProviderStreamError, as_bedrock_error, raise_provider_failure,
};
use crate::realtime_events::scope::EventScope;

// The seam's own exception type stands in for the SDK's here
type Failure = ProviderFailure<io::Error>;

pub(super) fn provider_failure_event() {
    mapped_classes();
    unknown_class();
    native_cause();
    redaction();
}

fn mapped_classes() {
    let expected = [
        ("AccessDeniedException", 403),
        ("ConflictException", 400),
        ("InternalServerException", 500),
        ("ModelErrorException", 424),
        ("ModelNotReadyException", 429),
        ("ModelStreamErrorException", 424),
        ("ModelTimeoutException", 408),
        ("ResourceNotFoundException", 404),
        ("ServiceQuotaExceededException", 400),
        ("ServiceUnavailableException", 503),
        ("ThrottlingException", 429),
        ("ValidationException", 400),
    ];
    for (class_name, status) in expected {
        let failure = Failure {
            class_name: class_name.to_owned(),
            message: "boom".to_owned(),
            source: None,
        };
        let error = as_bedrock_error(failure);
        match &error {
            ProviderStreamError::Bedrock {
                message,
                status_code,
                failure,
            } => {
                assert_eq!(*status_code, status);
                assert_eq!(message, &format!("{class_name}: boom"));
                assert_eq!(failure.class_name, class_name);
            }
            ProviderStreamError::Unchanged(failure) => {
                panic!("{} stayed unchanged", failure.class_name);
            }
        }
        assert!(Error::source(&error).is_some());
    }
}

fn unknown_class() {
    let error = as_bedrock_error(Failure {
        class_name: "Throttling".to_owned(),
        message: SECRET.to_owned(),
        source: None,
    });
    assert!(matches!(error, ProviderStreamError::Unchanged(_)));
    assert_eq!(error.to_string(), SECRET);
    assert!(Error::source(&error).is_none());
}

// The mapped error chain reaches the exception the seam caught, not just its message
fn native_cause() {
    let failure = Failure {
        class_name: "ThrottlingException".to_owned(),
        message: "slow down".to_owned(),
        source: Some(io::Error::other("connection reset")),
    };
    let error = as_bedrock_error(failure);
    let cause = Error::source(&error).expect("the provider failure");
    assert_eq!(cause.to_string(), "slow down");
    assert_eq!(
        cause.source().expect("the native cause").to_string(),
        "connection reset"
    );
}

fn redaction() {
    let (log, _guard) = support::capture();
    let mut scope = EventScope::default();
    scope.commit();
    let failure = Failure {
        class_name: "ThrottlingException".to_owned(),
        message: format!("upstream said {SECRET}"),
        source: None,
    };
    let error = raise_provider_failure(&mut scope, failure, &support::redactor());
    assert_eq!(
        error.to_string(),
        "ThrottlingException: upstream said REDACTED"
    );
    assert_eq!(
        scope.committed_failure,
        Json::Text("ThrottlingException: upstream said REDACTED".to_owned())
    );
    assert!(log.text().contains(
        "Bedrock Realtime: provider stream failed: ThrottlingException: upstream said REDACTED"
    ));
    assert!(!log.text().contains(SECRET));

    let mut uncommitted = EventScope::default();
    let failure = Failure {
        class_name: "ValidationException".to_owned(),
        message: "bad".to_owned(),
        source: None,
    };
    raise_provider_failure(&mut uncommitted, failure, &support::redactor());
    assert_eq!(uncommitted.committed_failure, Json::Null);
}
