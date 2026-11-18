use std::error::Error;

use futures_util::future::BoxFuture;

// proxy_logging_obj.alerting_handler behind a port, so the db crate never learns the alert channel's own error type.
pub trait OperatorAlert: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn send<'message>(
        &'message self,
        message: &'message str,
    ) -> BoxFuture<'message, Result<(), Self::Error>>;
}
