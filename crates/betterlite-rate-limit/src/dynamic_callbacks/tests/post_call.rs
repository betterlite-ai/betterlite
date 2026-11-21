#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::call_helpers::headers::AdditionalHeaders;

use super::tests::support::{
    CallbackFixture, Capture, headers, metadata_map, number, slot_released, stash_with_statuses,
    text,
};
use crate::dynamic_callbacks::{PriorityMetadata, post_call_success_hook};
use crate::limiter_records::{RateLimitStatus, RateLimitType};
use crate::post_call_settlement::success_hook::SuccessHook;

const PRIORITY_HEADER: &str = "x-litellm-priority";
const VERSION_HEADER: &str = "x-litellm-rate-limiter-version";

fn token_status(descriptor_key: &str, remaining: i64, limit: i64) -> RateLimitStatus {
    RateLimitStatus {
        code: "OK".to_owned(),
        current_limit: limit,
        limit_remaining: remaining,
        rate_limit_type: RateLimitType::Tokens,
        descriptor_key: descriptor_key.to_owned(),
        descriptor_value: None,
    }
}

async fn run_hook(
    key_metadata: &BTreeMap<String, JsonData>,
    team_metadata: Option<&BTreeMap<String, JsonData>>,
    response_headers: Option<&mut AdditionalHeaders>,
) -> bool {
    let fixture = CallbackFixture::new();
    let slot = stash_with_statuses(vec![token_status("model", 40, 100)]);
    let hook = SuccessHook {
        stash: &slot,
        call_id: None,
        response: None,
    };
    let priority = PriorityMetadata {
        team_metadata,
        key_metadata,
    };
    post_call_success_hook(&fixture.settlement(), &hook, &priority, response_headers).await;
    slot_released(&slot)
}

#[tokio::test]
async fn delegates_to_the_v3_limiter_before_writing_the_priority_headers() {
    let key_metadata = metadata_map(&[("priority", JsonData::String("medium".to_owned()))]);
    let mut response_headers = headers(&[("x-kept", text("unchanged"))]);
    let released = run_hook(&key_metadata, None, Some(&mut response_headers)).await;
    assert!(released);
    assert_eq!(
        response_headers.get("x-ratelimit-model-remaining-tokens"),
        Some(&number(40))
    );
    assert_eq!(
        response_headers.get("x-ratelimit-model-limit-tokens"),
        Some(&number(100))
    );
    assert_eq!(response_headers.get(PRIORITY_HEADER), Some(&text("medium")));
    assert_eq!(response_headers.get(VERSION_HEADER), Some(&text("v3")));
    assert_eq!(response_headers.get("x-kept"), Some(&text("unchanged")));
}

#[tokio::test]
async fn releases_the_slot_without_a_response_to_write_headers_into() {
    let key_metadata = metadata_map(&[("priority", JsonData::String("high".to_owned()))]);
    let released = run_hook(&key_metadata, None, None).await;
    assert!(released);
}

#[tokio::test]
async fn falls_back_to_the_default_priority_without_any_metadata() {
    let key_metadata = BTreeMap::new();
    let mut response_headers = AdditionalHeaders::new();
    assert!(run_hook(&key_metadata, None, Some(&mut response_headers)).await);
    assert_eq!(
        response_headers.get(PRIORITY_HEADER),
        Some(&text("default"))
    );
    assert_eq!(response_headers.get(VERSION_HEADER), Some(&text("v3")));
}

#[tokio::test]
async fn prefers_the_team_priority_over_the_key_priority() {
    let key_metadata = metadata_map(&[("priority", JsonData::String("low".to_owned()))]);
    let team_metadata = metadata_map(&[("priority", JsonData::String("high".to_owned()))]);
    let mut response_headers = AdditionalHeaders::new();
    assert!(
        run_hook(
            &key_metadata,
            Some(&team_metadata),
            Some(&mut response_headers)
        )
        .await
    );
    assert_eq!(response_headers.get(PRIORITY_HEADER), Some(&text("high")));
}

#[tokio::test]
async fn keeps_a_priority_at_the_last_latin1_code_point() {
    let key_metadata = metadata_map(&[("priority", JsonData::String("ÿ".to_owned()))]);
    let mut response_headers = AdditionalHeaders::new();
    assert!(run_hook(&key_metadata, None, Some(&mut response_headers)).await);
    assert_eq!(response_headers.get(PRIORITY_HEADER), Some(&text("ÿ")));
}

#[tokio::test]
async fn skips_a_priority_one_code_point_past_latin1() {
    let capture = Capture::start();
    let key_metadata = metadata_map(&[("priority", JsonData::String("Ā".to_owned()))]);
    let mut response_headers = AdditionalHeaders::new();
    assert!(run_hook(&key_metadata, None, Some(&mut response_headers)).await);
    assert!(response_headers.get(PRIORITY_HEADER).is_none());
    assert_eq!(response_headers.get(VERSION_HEADER), Some(&text("v3")));
    assert!(
        capture
            .messages()
            .iter()
            .any(|message| message.contains("not Latin-1 encodable")
                && message.contains("priority=Ā"))
    );
}
