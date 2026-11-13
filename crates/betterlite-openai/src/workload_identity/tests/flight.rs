use super::support::{Fixture, answer, millis, secs, token_reply};

const STATUS_500: &str = "Token exchange failed with status 500";

pub(super) async fn concurrent_refresh_single_flight() {
    shared_exchange().await;
    failed_exchange_is_not_shared().await;
    stale_waiters().await;
}

async fn shared_exchange() {
    let fixture = Fixture::new(vec![token_reply("shared", "3600")]);
    let config = fixture.config("flight", "subject");
    let all = tokio::join!(
        fixture.outcome(&config),
        fixture.outcome(&config),
        fixture.outcome(&config),
        fixture.outcome(&config),
        fixture.outcome(&config)
    );
    assert_eq!(all.0, "shared");
    assert_eq!(all.1, "shared");
    assert_eq!(all.2, "shared");
    assert_eq!(all.3, "shared");
    assert_eq!(all.4, "shared");
    assert_eq!(fixture.script.sent(), 1);
}

async fn failed_exchange_is_not_shared() {
    let fixture = Fixture::new(vec![answer(500, ""), token_reply("recovered", "3600")]);
    let config = fixture.config("flight", "subject");
    let (leader, follower) = tokio::join!(fixture.outcome(&config), fixture.outcome(&config));
    assert_eq!(leader, STATUS_500);
    assert_eq!(follower, "recovered");
    assert_eq!(fixture.script.sent(), 2);
}

async fn stale_waiters() {
    for (waited, expected) in [(millis(999_999), "old"), (secs(1000), "new")] {
        let replies = vec![
            token_reply("old", "1000"),
            answer(500, ""),
            token_reply("new", "1000"),
        ];
        let fixture = Fixture::new(replies);
        let config = fixture.config("flight", "subject");
        fixture.token(&config).await;
        fixture.clock.advance(waited);
        let (leader, follower) = tokio::join!(fixture.outcome(&config), fixture.outcome(&config));
        assert_eq!(leader, STATUS_500);
        assert_eq!(follower, expected);
    }
}
