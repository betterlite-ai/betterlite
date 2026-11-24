#![cfg(test)]

use betterlite_cache::backend::BackendError;
use betterlite_cache::cache_key::error::CacheKeyError;
use betterlite_cache::envelope::facade::WriteError;
use betterlite_cache::handler::HandlerError;
use betterlite_cache::types::cache_type::LiteLlmCacheType;

use super::CacheError;

#[test]
fn a_malformed_request_or_key_request_is_a_value_error() {
    assert!(CacheError::MissingResult.is_value_error());
    assert!(CacheError::MissingInput.is_value_error());
    assert!(
        CacheError::Handler {
            source: HandlerError::FieldType {
                field: "cache",
                expected: "an object",
            },
        }
        .is_value_error()
    );
    assert!(
        CacheError::Key {
            source: CacheKeyError::CachingGroups,
        }
        .is_value_error()
    );
}

#[test]
fn a_backend_failure_is_a_runtime_error() {
    assert!(!CacheError::FlushSize { size: 70_000 }.is_value_error());
    assert!(
        !CacheError::UnsupportedType {
            cache_type: LiteLlmCacheType::Redis,
        }
        .is_value_error()
    );
}

#[test]
fn an_unsupported_operation_is_not_implemented_but_a_broken_backend_is_not() {
    let unsupported = CacheError::Write {
        source: WriteError::Backend {
            source: BackendError::Unsupported("batch_cache_write"),
        },
    };
    assert!(unsupported.is_not_implemented());
    let broken = CacheError::Write {
        source: WriteError::Backend {
            source: BackendError::NotNumeric {
                key: "k".to_owned(),
            },
        },
    };
    assert!(!broken.is_not_implemented());
    assert!(!CacheError::MissingResult.is_not_implemented());
}
