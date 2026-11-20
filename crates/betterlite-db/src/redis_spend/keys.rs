// litellm/constants.py: the buffer list names, the fixed spend-log caps and the seven-slot pipeline order.
pub const REDIS_UPDATE_BUFFER_KEY: &str = "litellm_spend_update_buffer";
pub const REDIS_DAILY_USER_BUFFER_KEY: &str = "litellm_daily_spend_update_buffer";
pub const REDIS_DAILY_TEAM_BUFFER_KEY: &str = "litellm_daily_team_spend_update_buffer";
pub const REDIS_DAILY_ORG_BUFFER_KEY: &str = "litellm_daily_org_spend_update_buffer";
pub const REDIS_END_USER_BUFFER_KEY: &str = "litellm_daily_end_user_spend_update_buffer";
pub const REDIS_DAILY_AGENT_BUFFER_KEY: &str = "litellm_daily_agent_spend_update_buffer";
pub const REDIS_DAILY_TAG_BUFFER_KEY: &str = "litellm_daily_tag_spend_update_buffer";
pub const REDIS_WINDOW_BUFFER_KEY: &str = "litellm_window_spend_update_buffer";
pub const REDIS_SPEND_LOGS_BUFFER_KEY: &str = "litellm_spend_logs_buffer";
pub const REDIS_SPEND_LOGS_MAX_ROWS: i64 = 100_000;
pub const REDIS_SPEND_LOGS_DEQUEUE: i64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendBufferQueue {
    Entity,
    DailyUser,
    DailyTeam,
    DailyOrg,
    DailyEndUser,
    DailyAgent,
    Window,
    DailyTag,
}

impl SpendBufferQueue {
    // The slot order get_all_transactions_from_redis_buffer_pipeline pops in; the daily tag queue has its own reader.
    pub const PIPELINE: [Self; 7] = [
        Self::Entity,
        Self::DailyUser,
        Self::DailyTeam,
        Self::DailyOrg,
        Self::DailyEndUser,
        Self::DailyAgent,
        Self::Window,
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Entity => REDIS_UPDATE_BUFFER_KEY,
            Self::DailyUser => REDIS_DAILY_USER_BUFFER_KEY,
            Self::DailyTeam => REDIS_DAILY_TEAM_BUFFER_KEY,
            Self::DailyOrg => REDIS_DAILY_ORG_BUFFER_KEY,
            Self::DailyEndUser => REDIS_END_USER_BUFFER_KEY,
            Self::DailyAgent => REDIS_DAILY_AGENT_BUFFER_KEY,
            Self::Window => REDIS_WINDOW_BUFFER_KEY,
            Self::DailyTag => REDIS_DAILY_TAG_BUFFER_KEY,
        }
    }

    // ServiceTypes of litellm/types/services.py, the gauge label the queue-size event carries.
    #[must_use]
    pub const fn service(self) -> &'static str {
        match self {
            Self::Entity => "redis_spend_update_queue",
            Self::DailyUser => "redis_daily_spend_update_queue",
            Self::DailyTeam => "redis_daily_team_spend_update_queue",
            Self::DailyOrg => "redis_daily_org_spend_update_queue",
            Self::DailyEndUser => "redis_daily_end_user_spend_update_queue",
            Self::DailyAgent => "redis_daily_agent_spend_update_queue",
            Self::Window => "redis_window_spend_update_queue",
            Self::DailyTag => "redis_daily_tag_spend_update_queue",
        }
    }
}
