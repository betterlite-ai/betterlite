#![cfg(test)]

use crate::budget_reset_policy::cache_invalidation::InvalidatedCache;
use crate::budget_tier_sweep::enduser_invalidation::{
    EndUserWalkRequest, invalidate_enduser_caches,
};
use crate::budget_tier_sweep::enduser_page_truncation::EndUserWalk;
use crate::budget_tier_sweep::tests::support::{CountingReconnect, RecordingCache, finish, schema};

#[test]
fn a_full_page_moves_the_cursor_after_its_last_row() {
    let walk = EndUserWalk {
        cursor: Some(String::new()),
        invalidated: 0,
        truncated: false,
    }
    .after_page(Some("u2"), 2, 2);
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: Some("u2".to_owned()),
            invalidated: 2,
            truncated: false,
        }
    );
}

#[test]
fn a_page_over_the_batch_keeps_the_cursor() {
    let walk = EndUserWalk {
        cursor: Some(String::new()),
        invalidated: 0,
        truncated: false,
    }
    .after_page(Some("u9"), 3, 2);
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: Some("u9".to_owned()),
            invalidated: 3,
            truncated: false,
        }
    );
}

#[test]
fn a_short_page_ends_the_walk() {
    let walk = EndUserWalk {
        cursor: Some(String::new()),
        invalidated: 0,
        truncated: false,
    }
    .after_page(Some("u2"), 2, 2)
    .after_page(Some("u5"), 1, 2);
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: None,
            invalidated: 3,
            truncated: false,
        }
    );
}

#[test]
fn an_empty_page_ends_the_walk_with_the_reached_count() {
    let walk = EndUserWalk {
        cursor: Some(String::new()),
        invalidated: 0,
        truncated: false,
    }
    .after_page(Some("u2"), 2, 2)
    .after_page(None, 0, 2);
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: None,
            invalidated: 2,
            truncated: false,
        }
    );
}

#[test]
fn a_failed_page_drops_the_cursor_and_marks_the_walk_truncated() {
    let walk = EndUserWalk {
        cursor: Some(String::new()),
        invalidated: 0,
        truncated: false,
    }
    .after_page(Some("u2"), 2, 2)
    .truncated_at();
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: None,
            invalidated: 2,
            truncated: true,
        }
    );
}

#[tokio::test]
async fn a_failed_page_stops_the_walk_past_the_customers_it_reached() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('u1', 'tier-a', 10), ('u2', 'tier-a', 10), ('u3', 'tier-a', 10), ('u4', 'tier-a', 10)",
        )
        .await
        .unwrap();
    let cache = RecordingCache::sabotaging(store.clone());
    let budget_ids = vec![String::from("tier-a")];
    let request = EndUserWalkRequest {
        budget_ids: &budget_ids,
        default_enduser_budget_id: None,
        batch_size: 2,
    };
    let mut reconnect = CountingReconnect::new(false);
    let walk = invalidate_enduser_caches(&store, &cache, request, Some(&mut reconnect)).await;
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: None,
            invalidated: 2,
            truncated: true,
        }
    );
    let calls = cache.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert_eq!(
        calls[0].keys,
        vec![
            "spend:end_user:u1".to_owned(),
            "spend:end_user:u2".to_owned()
        ]
    );
    finish(fixture, store, client).await;
}
