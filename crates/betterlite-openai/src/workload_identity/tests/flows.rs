use serde::Deserialize;

use super::support::{Fixture, Reply, answer, millis, secs, token_reply};
use crate::workload_identity::config::{
    OPENAI_WIF_CLIENT_ID, OpenAiWorkloadIdentityConfig, SdkCapability,
};
use crate::workload_identity::exchange::TOKEN_EXCHANGE_URL;

const STATUS_500: &str = "Token exchange failed with status 500";
const UNUSABLE: &str = "Token is unusable after refresh completed";

#[derive(Deserialize)]
struct Body {
    subject_token: String,
    identity_provider_id: String,
    service_account_id: String,
}

pub(super) async fn kubernetes_token_exchange() {
    let fixture = Fixture::new(vec![token_reply("access-1", "3600")]);
    let config = fixture.config("k8s", "  jwt-subject\r\n");
    let sdk = config
        .to_sdk_workload_identity(SdkCapability::AVAILABLE)
        .unwrap();
    assert_eq!(sdk.client_id, OPENAI_WIF_CLIENT_ID);
    assert_eq!(sdk.client_id, "litellm");
    assert_eq!(sdk.token_file, config.token_file);
    assert_eq!(fixture.token(&config).await, "access-1");
    let sent = fixture.script.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, TOKEN_EXCHANGE_URL);
    assert_eq!(sent[0].url, "https://auth.openai.com/oauth/token");
    assert!(sent[0].json);
    assert_eq!(sent[0].deadline, secs(10));
    let body: Body = serde_json::from_str(&sent[0].body).unwrap();
    assert_eq!(body.subject_token, "jwt-subject");
    assert_eq!(body.identity_provider_id, "idp-k8s");
    assert_eq!(body.service_account_id, "sa-k8s");
}

pub(super) async fn token_refresh_and_error() {
    let fixture = Fixture::new(vec![
        token_reply("first", "3600"),
        answer(500, ""),
        token_reply("second", "3600"),
        Reply::Failure,
        token_reply("third", "100"),
    ]);
    let config = fixture.config("refresh", "subject");
    assert_eq!(fixture.outcome(&config).await, "first");
    fixture.clock.advance(secs(2399));
    assert_eq!(fixture.outcome(&config).await, "first");
    assert_eq!(fixture.script.sent(), 1);
    fixture.clock.advance(secs(1));
    assert_eq!(fixture.outcome(&config).await, STATUS_500);
    assert_eq!(fixture.script.sent(), 2);
    recover_after_failures(&fixture, &config).await;
}

async fn recover_after_failures(fixture: &Fixture, config: &OpenAiWorkloadIdentityConfig) {
    assert_eq!(fixture.outcome(config).await, "second");
    fixture.clock.advance(secs(3600));
    assert_eq!(
        fixture.outcome(config).await,
        "the token exchange request failed"
    );
    assert_eq!(fixture.outcome(config).await, "third");
    assert_eq!(fixture.script.sent(), 5);
}

pub(super) async fn refresh_half_life_boundary() {
    let lifetimes = ["1000", "1000", "4000", "4000", "1", "1", "0.5", "0", "-5"];
    let fixture = Fixture::new(
        lifetimes
            .map(|expires| token_reply("token", expires))
            .to_vec(),
    );
    let config = fixture.config("half-life", "subject");
    let steps = [
        (millis(0), 1),
        (millis(499_999), 1),
        (millis(1), 2),
        (secs(2799), 3),
        (secs(2799), 3),
        (secs(1), 4),
        (secs(4000), 5),
        (millis(499), 5),
        (millis(1), 6),
        (secs(1), 7),
    ];
    for (advance, sent) in steps {
        fixture.clock.advance(advance);
        fixture.token(&config).await;
        assert_eq!(fixture.script.sent(), sent, "after {advance:?}");
    }
    fixture.clock.advance(secs(1));
    for sent in [8_usize, 9_usize] {
        assert_eq!(fixture.outcome(&config).await, UNUSABLE);
        assert_eq!(fixture.script.sent(), sent);
    }
}
