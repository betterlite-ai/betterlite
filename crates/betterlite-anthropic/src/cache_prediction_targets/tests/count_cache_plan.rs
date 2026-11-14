use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tokio::sync::Barrier;
use tokio::time::timeout;

use super::support::{KEY, MODEL, ScriptedCounter, json, secret};
use crate::cache_plan::{PromptCachePlan, UnsupportedCachePlan, parse_cache_plan};
use crate::cache_prediction_targets::counting::TokenCounter;
use crate::cache_prediction_targets::plan_counting::{ThinkingCacheSupport, count_cache_plan};

const SINGLE: &str = r#"{"model":"claude-x","max_tokens":5,"messages":[{"role":"user","content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}]},{"role":"assistant","content":"world"}]}"#;
const TWO: &str = r#"{"model":"claude-x","max_tokens":5,"messages":[{"role":"user","content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}]},{"role":"assistant","content":[{"type":"text","text":"world","cache_control":{"type":"ephemeral"}}]},{"role":"user","content":"again"}]}"#;
const THINKING: &str = r#"{"model":"claude-x","max_tokens":5,"messages":[{"role":"user","content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}]},{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"}]},{"role":"user","content":[{"type":"text","text":"again","cache_control":{"type":"ephemeral"}}]}]}"#;
const PLAIN: &str =
    r#"{"model":"claude-x","max_tokens":5,"messages":[{"role":"user","content":"hello"}]}"#;

struct Recording {
    asked: Mutex<Vec<String>>,
    supported: bool,
}

impl Recording {
    fn new(supported: bool) -> Self {
        Self {
            asked: Mutex::new(Vec::new()),
            supported,
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ThinkingCacheSupport for Recording {
    fn preserves_thinking_cache(&self, model: &str) -> impl Future<Output = bool> + Send {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        ready(self.supported)
    }
}

struct ConcurrentCounter {
    calls: AtomicUsize,
    barrier: Barrier,
}

impl TokenCounter for ConcurrentCounter {
    async fn count(&self, _model: &str, _api_key: &SecretString, _body: &Json) -> Option<i64> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Some(30);
        }
        self.barrier.wait().await;
        Some(10)
    }
}

fn plan(body: &str) -> PromptCachePlan {
    parse_cache_plan(body).expect("cache plan")
}

async fn counted(
    plan: &PromptCachePlan,
    replies: &[Option<i64>],
    supported: bool,
) -> (
    Result<CountedPromptCachePlan, UnsupportedCachePlan>,
    ScriptedCounter,
    Recording,
) {
    let counter = ScriptedCounter::new(replies);
    let thinking = Recording::new(supported);
    let key = secret(KEY);
    let result = count_cache_plan(MODEL, &key, plan, &counter, &thinking).await;
    (result, counter, thinking)
}

fn expected(plan: &PromptCachePlan, total: i64, counts: [i64; 2]) -> CountedPromptCachePlan {
    CountedPromptCachePlan {
        total_tokens: total,
        breakpoints: plan
            .breakpoints
            .iter()
            .zip(counts)
            .map(|(marker, prefix_tokens)| CountedBreakpoint {
                fingerprint: marker.fingerprint.clone(),
                ttl_seconds: 300,
                prefix_tokens,
                lookback_fingerprints: marker.lookback_fingerprints.clone(),
                content_fingerprint: marker.content_fingerprint.clone(),
                lookback_content_fingerprints: marker.lookback_content_fingerprints.clone(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn count_cache_plan_counts_every_breakpoint() {
    let plan = plan(TWO);
    assert_eq!(plan.breakpoints.len(), 2);
    let (result, _counter, _thinking) = counted(&plan, &[Some(30), Some(10), Some(20)], true).await;
    assert_eq!(result.unwrap(), expected(&plan, 30, [10, 20]));
}

#[tokio::test]
async fn count_cache_plan_counts_the_whole_body_before_the_prefixes() {
    let plan = plan(TWO);
    let (result, counter, _thinking) = counted(&plan, &[Some(30), Some(10), Some(20)], true).await;
    assert_eq!(result.unwrap().total_tokens, 30);
    let bodies = counter.counted_bodies();
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[0], json(TWO));
}

#[tokio::test]
async fn count_cache_plan_counts_a_plan_without_breakpoints() {
    let plan = plan(PLAIN);
    assert!(plan.breakpoints.is_empty());
    let (result, counter, _thinking) = counted(&plan, &[Some(30)], true).await;
    assert_eq!(
        result.unwrap(),
        CountedPromptCachePlan {
            total_tokens: 30,
            breakpoints: Vec::new(),
        }
    );
    assert_eq!(counter.counted_bodies().len(), 1);
}

#[tokio::test]
async fn count_cache_plan_rejects_thinking_blocks_without_preservation() {
    let (result, counter, thinking) =
        counted(&plan(THINKING), &[Some(100), Some(40), Some(80)], false).await;
    assert_eq!(result, Err(UnsupportedCachePlan::UnsupportedThinkingCache));
    assert_eq!(thinking.asked(), vec![MODEL.to_owned()]);
    assert!(counter.counted_bodies().is_empty());
}

#[tokio::test]
async fn count_cache_plan_counts_thinking_blocks_when_preserved() {
    let (result, _counter, thinking) =
        counted(&plan(THINKING), &[Some(100), Some(40), Some(80)], true).await;
    assert_eq!(result.unwrap().total_tokens, 100);
    assert_eq!(thinking.asked(), vec![MODEL.to_owned()]);
}

#[tokio::test]
async fn count_cache_plan_skips_the_thinking_check_without_thinking_blocks() {
    let (result, _counter, thinking) = counted(&plan(SINGLE), &[Some(30), Some(10)], false).await;
    assert_eq!(result.unwrap().total_tokens, 30);
    assert!(thinking.asked().is_empty());
}

#[tokio::test]
async fn count_cache_plan_returns_unavailable_when_the_total_fails() {
    let (result, counter, _thinking) = counted(&plan(SINGLE), &[None], true).await;
    assert_eq!(result, Err(UnsupportedCachePlan::TokenCountUnavailable));
    assert_eq!(counter.counted_bodies().len(), 1);
}

#[tokio::test]
async fn count_cache_plan_returns_unavailable_when_a_prefix_fails() {
    let (result, counter, _thinking) = counted(&plan(TWO), &[Some(30), Some(10), None], true).await;
    assert_eq!(result, Err(UnsupportedCachePlan::TokenCountUnavailable));
    assert_eq!(counter.counted_bodies().len(), 3);
}

#[rstest::rstest]
#[case::negative_prefix(&[Some(30), Some(-1)])]
#[case::negative_total(&[Some(-1), Some(10)])]
#[tokio::test]
async fn count_cache_plan_rejects_a_negative_count(#[case] replies: &[Option<i64>]) {
    let (result, _counter, _thinking) = counted(&plan(SINGLE), replies, true).await;
    assert_eq!(
        result,
        Err(UnsupportedCachePlan::InconsistentPrefixTokenCount)
    );
}

#[rstest::rstest]
#[case::rising_prefix(&[Some(30), Some(20), Some(10)])]
#[case::prefix_above_the_total(&[Some(30), Some(10), Some(40)])]
#[tokio::test]
async fn count_cache_plan_rejects_inconsistent_prefix_counts(#[case] replies: &[Option<i64>]) {
    let (result, _counter, _thinking) = counted(&plan(TWO), replies, true).await;
    assert_eq!(
        result,
        Err(UnsupportedCachePlan::InconsistentPrefixTokenCount)
    );
}

#[tokio::test]
async fn count_cache_plan_accepts_a_zero_count() {
    let plan = plan(SINGLE);
    let (result, _counter, _thinking) = counted(&plan, &[Some(0), Some(0)], true).await;
    let counted = result.unwrap();
    assert_eq!(counted.total_tokens, 0);
    assert_eq!(counted.breakpoints[0].prefix_tokens, 0);
}

#[tokio::test]
async fn count_cache_plan_accepts_equal_counts() {
    let plan = plan(TWO);
    let (result, _counter, _thinking) = counted(&plan, &[Some(10), Some(10), Some(10)], true).await;
    assert_eq!(result.unwrap(), expected(&plan, 10, [10, 10]));
}

#[tokio::test(start_paused = true)]
async fn count_cache_plan_counts_breakpoints_concurrently() {
    let plan = plan(TWO);
    let counter = ConcurrentCounter {
        calls: AtomicUsize::new(0),
        barrier: Barrier::new(2),
    };
    let thinking = Recording::new(true);
    let key = secret(KEY);
    let result = timeout(
        Duration::from_secs(1),
        count_cache_plan(MODEL, &key, &plan, &counter, &thinking),
    )
    .await;
    assert!(matches!(result, Ok(Ok(_))));
}
