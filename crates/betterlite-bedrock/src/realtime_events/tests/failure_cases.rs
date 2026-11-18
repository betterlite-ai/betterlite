use std::error::Error;

use betterlite_models::decisions::Json;

use super::support::{self, SECRET};
use crate::realtime_events::failure::{
    ProviderFailure, ProviderStreamError, as_bedrock_error, raise_provider_failure,
};
use crate::realtime_events::scope::EventScope;

pub(super) fn provider_failure_event() {
    mapped_classes();
    unknown_class();
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
        let failure = ProviderFailure {
            class_name: class_name.to_owned(),
            message: "boom".to_owned(),
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
    let error = as_bedrock_error(ProviderFailure {
        class_name: "Throttling".to_owned(),
        message: SECRET.to_owned(),
    });
    assert!(matches!(error, ProviderStreamError::Unchanged(_)));
    assert_eq!(error.to_string(), SECRET);
    assert!(Error::source(&error).is_none());
}

fn redaction() {
    let (log, _guard) = support::capture();
    let mut scope = EventScope::default();
    scope.commit();
    let failure = ProviderFailure {
        class_name: "ThrottlingException".to_owned(),
        message: format!("upstream said {SECRET}"),
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
    let failure = ProviderFailure {
        class_name: "ValidationException".to_owned(),
        message: "bad".to_owned(),
    };
    raise_provider_failure(&mut uncommitted, failure, &support::redactor());
    assert_eq!(uncommitted.committed_failure, Json::Null);
}
