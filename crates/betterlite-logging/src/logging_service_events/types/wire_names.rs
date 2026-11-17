use super::ServiceType;

impl ServiceType {
    // LiteLLM's ServiceTypes is a str enum, and these are the values it writes into a payload
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::BatchWriteToDb => "batch_write_to_db",
            Self::ResetBudgetJob => "reset_budget_job",
            Self::Litellm => "self",
            Self::Router => "router",
            Self::Auth => "auth",
            Self::ProxyPreCall => "proxy_pre_call",
            Self::PodLockManager => "pod_lock_manager",
            Self::InMemoryDailySpendUpdateQueue => "in_memory_daily_spend_update_queue",
            Self::RedisDailySpendUpdateQueue => "redis_daily_spend_update_queue",
            Self::RedisDailyEndUserQueue => "redis_daily_end_user_spend_update_queue",
            Self::RedisDailyOrgSpendUpdateQueue => "redis_daily_org_spend_update_queue",
            Self::RedisDailyTeamSpendUpdateQueue => "redis_daily_team_spend_update_queue",
            Self::RedisDailyAgentQueue => "redis_daily_agent_spend_update_queue",
            Self::RedisDailyTagSpendUpdateQueue => "redis_daily_tag_spend_update_queue",
            Self::InMemorySpendUpdateQueue => "in_memory_spend_update_queue",
            Self::RedisSpendUpdateQueue => "redis_spend_update_queue",
            Self::RedisWindowSpendUpdateQueue => "redis_window_spend_update_queue",
        }
    }
}
