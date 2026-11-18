use betterlite_db::ptu::rollup::alert::OperatorAlert;
use futures_util::future::BoxFuture;

use crate::failure_hook::alerts::{AlertError, ProxyAlerts};
use crate::failure_hook::ports::{AlertLevel, AlertType};

// _alert_ptu_rollup_failure: the spend writer's alerting_handler at High with failed_tracking_spend.
impl OperatorAlert for ProxyAlerts {
    type Error = AlertError;

    fn send<'message>(
        &'message self,
        message: &'message str,
    ) -> BoxFuture<'message, Result<(), AlertError>> {
        Box::pin(self.alerting_handler(
            message,
            AlertLevel::High,
            AlertType::FailedTrackingSpend,
            None,
        ))
    }
}
