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

// One queue's litellm/constants.py list name and the ServiceTypes label its queue-size event carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendBufferNames {
    pub key: &'static str,
    pub service: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
    pub const fn names(self) -> SpendBufferNames {
        match self {
            Self::Entity => SpendBufferNames {
                key: REDIS_UPDATE_BUFFER_KEY,
                service: "redis_spend_update_queue",
            },
            Self::DailyUser => SpendBufferNames {
                key: REDIS_DAILY_USER_BUFFER_KEY,
                service: "redis_daily_spend_update_queue",
            },
            Self::DailyTeam => SpendBufferNames {
                key: REDIS_DAILY_TEAM_BUFFER_KEY,
                service: "redis_daily_team_spend_update_queue",
            },
            Self::DailyOrg => SpendBufferNames {
                key: REDIS_DAILY_ORG_BUFFER_KEY,
                service: "redis_daily_org_spend_update_queue",
            },
            Self::DailyEndUser => SpendBufferNames {
                key: REDIS_END_USER_BUFFER_KEY,
                service: "redis_daily_end_user_spend_update_queue",
            },
            Self::DailyAgent => SpendBufferNames {
                key: REDIS_DAILY_AGENT_BUFFER_KEY,
                service: "redis_daily_agent_spend_update_queue",
            },
            Self::Window => SpendBufferNames {
                key: REDIS_WINDOW_BUFFER_KEY,
                service: "redis_window_spend_update_queue",
            },
            Self::DailyTag => SpendBufferNames {
                key: REDIS_DAILY_TAG_BUFFER_KEY,
                service: "redis_daily_tag_spend_update_queue",
            },
        }
    }
}
