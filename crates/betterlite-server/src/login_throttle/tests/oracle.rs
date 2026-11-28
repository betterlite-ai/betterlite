use std::collections::BTreeMap;

use betterlite_config::sections::login_throttle::Raw;
use betterlite_models::decisions::Json;

use crate::login_throttle::support::{
    OracleInput, Phase, RedisSpec, RequestSpec, Step, run_oracle,
};

fn request(peer: Option<&str>, forwarded: Option<&str>) -> RequestSpec {
    RequestSpec {
        peer: peer.map(str::to_owned),
        x_forwarded_for: forwarded.map(str::to_owned),
        host: Some("proxy.internal".to_owned()),
    }
}

fn step(kind: &str, username: Option<&str>) -> Step {
    Step {
        kind: kind.to_owned(),
        username: username.map(str::to_owned),
    }
}

fn config(text: &str) -> Raw {
    serde_json::from_str(text).expect("the config")
}

fn json_of(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected json")
}

fn rendered(input: &OracleInput) -> Json {
    let text = serde_json::to_string(&run_oracle(input).expect("the oracle run"))
        .expect("the output text");
    serde_json::from_str(&text).expect("the output json")
}

const ALLOWED: &str = r#"{"step": "attempt", "outcome": "allowed", "error": null, "user_block": null, "source_block": null}"#;
const FAILED: &str = r#"{"step": "failed", "outcome": "ok", "error": null, "user_block": null, "source_block": null}"#;
const SUCCEEDED: &str = r#"{"step": "succeeded", "outcome": "ok", "error": null, "user_block": null, "source_block": null}"#;

fn throttle(client_ip: &str, source_limit: &str) -> String {
    format!(
        r#"{{"client_ip": "{client_ip}", "source_limit": {source_limit}, "user_limit": 5, "window_seconds": 60, "block_seconds": 300, "enabled": true}}"#
    )
}

#[test]
fn oracle_replays_every_step_with_a_shared_store() {
    let input = OracleInput {
        config: config(
            r#"{"general_settings": {"max_failed_login_attempts_per_source": 4, "trusted_proxy_ranges": [], "failed_login_window_seconds": 60, "failed_login_block_seconds": 300}}"#,
        ),
        redis: Some(RedisSpec {
            faults: BTreeMap::new(),
        }),
        phases: vec![
            Phase {
                request: request(Some("10.1.2.3"), Some("203.0.113.9")),
                now: 1000.0,
                steps: vec![
                    step("attempt", Some("bob")),
                    step("failed", None),
                    step("attempt", Some("bob")),
                    step("succeeded", None),
                    step("record_failure", Some("bob")),
                    step("clear_pair", Some("bob")),
                    step("other", None),
                ],
            },
            Phase {
                request: request(Some("10.1.2.3"), Some("203.0.113.9")),
                now: 1005.0,
                steps: vec![
                    step("attempt", Some("bob")),
                    step("failed", None),
                    step("attempt", Some("bob")),
                    step("failed", None),
                    step("attempt", Some("bob")),
                    step("failed", None),
                    step("attempt", Some("bob")),
                ],
            },
        ],
    };
    let throttle_view = r#"{"client_ip": "10.1.2.3", "source_limit": 4, "user_limit": 2, "window_seconds": 60, "block_seconds": 300, "enabled": true}"#;
    let expected = json_of(&format!(
        r#"{{"phases": [
            {{
                "throttle": {throttle_view},
                "steps": [
                    {ALLOWED},
                    {FAILED},
                    {ALLOWED},
                    {SUCCEEDED},
                    {{"step": "record_failure", "outcome": "ok", "error": null, "user_block": 0, "source_block": 0}},
                    {{"step": "clear_pair", "outcome": "ok", "error": null, "user_block": null, "source_block": null}},
                    {{"step": "other", "outcome": "unknown", "error": null, "user_block": null, "source_block": null}}
                ],
                "warnings": []
            }},
            {{
                "throttle": {throttle_view},
                "steps": [
                    {ALLOWED},
                    {FAILED},
                    {ALLOWED},
                    {FAILED},
                    {ALLOWED},
                    {FAILED},
                    {{"step": "attempt", "outcome": "refused", "error": {{"code": "429", "type": "auth_error", "param": "username", "headers": {{"Retry-After": "300"}}}}, "user_block": null, "source_block": null}}
                ],
                "warnings": ["blocked", "refused"]
            }}
        ]}}"#
    ));
    assert_eq!(rendered(&input), expected);
}

#[test]
fn oracle_reports_shared_store_faults() {
    let faults: BTreeMap<String, String> = [
        ("block_ttls".to_owned(), "malformed".to_owned()),
        ("record_failure".to_owned(), "malformed".to_owned()),
        ("delete".to_owned(), "unavailable".to_owned()),
    ]
    .into_iter()
    .collect();
    let input = OracleInput {
        config: config("{}"),
        redis: Some(RedisSpec { faults }),
        phases: vec![Phase {
            request: request(Some("10.1.2.3"), None),
            now: 1000.0,
            steps: vec![
                step("attempt", Some("bob")),
                step("record_failure", Some("bob")),
                step("clear_pair", Some("bob")),
            ],
        }],
    };
    let expected = json_of(&format!(
        r#"{{"phases": [{{
            "throttle": {},
            "steps": [
                {{"step": "attempt", "outcome": "error", "error": null, "user_block": null, "source_block": null}},
                {{"step": "record_failure", "outcome": "error", "error": null, "user_block": null, "source_block": null}},
                {{"step": "clear_pair", "outcome": "ok", "error": null, "user_block": null, "source_block": null}}
            ],
            "warnings": ["redis_fallback"]
        }}]}}"#,
        throttle("10.1.2.3", "null"),
    ));
    assert_eq!(rendered(&input), expected);
}

#[test]
fn oracle_without_a_shared_store() {
    let input = OracleInput {
        config: config("{}"),
        redis: None,
        phases: vec![Phase {
            request: RequestSpec::default(),
            now: 1000.0,
            steps: vec![
                step("failed", None),
                step("attempt", Some("bob")),
                step("failed", None),
                step("attempt", Some("bob")),
                step("succeeded", None),
                step("record_failure", Some("bob")),
            ],
        }],
    };
    let expected = json_of(&format!(
        r#"{{"phases": [{{
            "throttle": {},
            "steps": [
                {{"step": "failed", "outcome": "no_attempt", "error": null, "user_block": null, "source_block": null}},
                {ALLOWED},
                {FAILED},
                {ALLOWED},
                {SUCCEEDED},
                {{"step": "record_failure", "outcome": "ok", "error": null, "user_block": 0, "source_block": 0}}
            ],
            "warnings": []
        }}]}}"#,
        throttle("unknown", "null"),
    ));
    assert_eq!(rendered(&input), expected);
}
