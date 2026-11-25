use std::fmt::Display;

// Each helper runs inside its own catch, so one failure never skips the siblings that come after it.
pub(super) fn isolate<E>(label: &'static str, result: Result<(), E>)
where
    E: Display,
{
    if let Err(error) = result {
        tracing::debug!(
            helper = label,
            error = %error,
            "_batch_database_updates: the helper failed"
        );
    }
}
