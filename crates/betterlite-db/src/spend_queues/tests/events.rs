use crate::spend_queues::events::SpendQueueService;

#[test]
fn service_kinds_carry_the_source_names() {
    assert_eq!(
        SpendQueueService::InMemorySpendUpdateQueue.as_str(),
        "in_memory_spend_update_queue"
    );
    assert_eq!(
        SpendQueueService::InMemoryDailySpendUpdateQueue.as_str(),
        "in_memory_daily_spend_update_queue"
    );
}
