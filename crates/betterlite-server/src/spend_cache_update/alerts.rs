use async_trait::async_trait;

use crate::failure_hook::alerts::ProxyAlerts;
use crate::failure_hook::ports::{BudgetAlertKind, CallInfo};

#[async_trait]
pub trait SpendBudgetAlerts: Send + Sync {
    async fn projected_limit_exceeded(&self, info: CallInfo);
}

#[async_trait]
impl SpendBudgetAlerts for ProxyAlerts {
    async fn projected_limit_exceeded(&self, info: CallInfo) {
        self.budget_alerts(BudgetAlertKind::ProjectedLimitExceeded, &info)
            .await;
    }
}
