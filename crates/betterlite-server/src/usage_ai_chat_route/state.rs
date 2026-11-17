use std::sync::Arc;

use chrono::NaiveDate;

use crate::keepalive_interval::PingSetting;
use crate::usage_ai_chat::model::UsageChatModel;
use crate::usage_ai_chat::parts::UsageChatParts;
use crate::usage_ai_chat::source::UsageDataSource;

use super::caller::NonAdminCallerId;

pub trait UsageChatFactory: Send + Sync {
    type Model: UsageChatModel + Send + 'static;
    type Data: UsageDataSource + Send + 'static;
    type Clock: Fn() -> NaiveDate + Send + Sync + 'static;

    fn parts(&self) -> UsageChatParts<Self::Model, Self::Data, Self::Clock>;
}

pub struct UsageAiChatState<F> {
    pub factory: Arc<F>,
    pub ping_interval: Option<PingSetting>,
    pub caller_id: Arc<dyn NonAdminCallerId>,
}

impl<F> Clone for UsageAiChatState<F> {
    fn clone(&self) -> Self {
        Self {
            factory: Arc::clone(&self.factory),
            ping_interval: self.ping_interval.clone(),
            caller_id: Arc::clone(&self.caller_id),
        }
    }
}
