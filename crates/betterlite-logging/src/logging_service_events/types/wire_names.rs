use super::ServiceType;

// LiteLLM's ServiceTypes is a str enum, and these are the values it writes into a payload
#[must_use]
pub(crate) const fn as_str(service: ServiceType) -> &'static str {
    match service {
        ServiceType::Redis => "redis",
        ServiceType::Postgres => "postgres",
        ServiceType::BatchWriteToDb => "batch_write_to_db",
        ServiceType::ResetBudgetJob => "reset_budget_job",
        ServiceType::Litellm => "self",
        ServiceType::Router => "router",
        ServiceType::Auth => "auth",
        ServiceType::ProxyPreCall => "proxy_pre_call",
        ServiceType::PodLockManager => "pod_lock_manager",
        ServiceType::InMemoryDailySpendUpdateQueue => "in_memory_daily_spend_update_queue",
        ServiceType::RedisDailySpendUpdateQueue => "redis_daily_spend_update_queue",
        ServiceType::RedisDailyEndUserQueue => "redis_daily_end_user_spend_update_queue",
        ServiceType::RedisDailyOrgSpendUpdateQueue => "redis_daily_org_spend_update_queue",
        ServiceType::RedisDailyTeamSpendUpdateQueue => "redis_daily_team_spend_update_queue",
        ServiceType::RedisDailyAgentQueue => "redis_daily_agent_spend_update_queue",
        ServiceType::RedisDailyTagSpendUpdateQueue => "redis_daily_tag_spend_update_queue",
        ServiceType::InMemorySpendUpdateQueue => "in_memory_spend_update_queue",
        ServiceType::RedisSpendUpdateQueue => "redis_spend_update_queue",
        ServiceType::RedisWindowSpendUpdateQueue => "redis_window_spend_update_queue",
    }
}
