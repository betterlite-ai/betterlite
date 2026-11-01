use crate::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use crate::baseline_history::model::CacheEntry;

pub(crate) fn is_plan_valid(plan: Option<&CountedPromptCachePlan>) -> bool {
    let Some(candidate) = plan else { return false };
    if candidate.total_tokens < 0 || candidate.breakpoints.len() > 4 {
        return false;
    }
    candidate.breakpoints.iter().all(|marker| {
        !marker.fingerprint.is_empty()
            && !marker.content_fingerprint.is_empty()
            && marker.lookback_fingerprints.contains(&marker.fingerprint)
            && marker
                .lookback_content_fingerprints
                .contains(&marker.content_fingerprint)
            && matches!(marker.ttl_seconds, 300 | 3600)
            && (0..=candidate.total_tokens).contains(&marker.prefix_tokens)
    }) && candidate.breakpoints.windows(2).all(|pair| {
        let [left, right] = pair else { return true };
        left.prefix_tokens <= right.prefix_tokens && left.ttl_seconds >= right.ttl_seconds
    })
}

pub(crate) fn markers_for(
    plan: &CountedPromptCachePlan,
    minimum_cache_tokens: i64,
) -> Vec<&CountedBreakpoint> {
    let mut markers = Vec::new();
    for marker in &plan.breakpoints {
        if marker.prefix_tokens >= minimum_cache_tokens {
            markers.push(marker);
        }
    }
    markers
}

pub(crate) struct EntryStatus {
    pub matches: bool,
    pub ambiguous: bool,
}

pub(crate) fn entry_status(
    entry: &CacheEntry,
    markers: &[&CountedBreakpoint],
    started: f64,
) -> EntryStatus {
    if entry.available_at > started || started >= entry.expires_at {
        return EntryStatus {
            matches: false,
            ambiguous: false,
        };
    }
    let matches = markers.iter().any(|marker| {
        marker.lookback_fingerprints.contains(&entry.fingerprint)
            && entry.tokens <= marker.prefix_tokens
            && entry.ttl_seconds == marker.ttl_seconds
    });
    let ambiguous = markers.iter().any(|marker| {
        marker
            .lookback_content_fingerprints
            .contains(&entry.content_fingerprint)
            && (entry.uncertain || entry.ttl_seconds != marker.ttl_seconds)
    });
    EntryStatus { matches, ambiguous }
}
