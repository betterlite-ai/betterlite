// Each helper runs inside its own catch, so one failure never skips the siblings that come after it.
macro_rules! isolated {
    ($label:literal, $call:expr) => {
        if let Err(error) = $call {
            tracing::debug!(
                helper = $label,
                error = %error,
                "_batch_database_updates: the helper failed"
            );
        }
    };
}

pub(super) use isolated;
