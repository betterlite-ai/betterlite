use tracing::warn;

use crate::cache_accounting::constants::MAX_CACHE_CONTROL_BLOCKS;
use crate::cache_accounting::control::default_control;
use crate::cache_accounting::points::MessageInjectionPoint;
use crate::cache_placement::insert::mark_message;
use crate::cache_placement::marks::{count_message_marks, count_request_marks};
use crate::cache_placement::targets::resolve_target_indices;
use crate::message_content::Message;

#[must_use]
pub fn apply_message_injections(
    points: &[&MessageInjectionPoint],
    messages: &[Message],
    max_blocks: usize,
    openai_dialect: bool,
) -> Vec<Message> {
    let mut processed = messages.to_vec();
    let mut used_blocks = count_request_marks(&processed, None);
    let mut limit_reached = false;
    for point in points.iter().copied() {
        if used_blocks >= max_blocks {
            limit_reached = true;
            break;
        }
        let control = point
            .control
            .clone()
            .unwrap_or_else(|| default_control(None));
        for target in resolve_target_indices(point, &processed) {
            if used_blocks >= max_blocks {
                limit_reached = true;
                break;
            }
            let Some(slot) = processed.get_mut(target) else {
                continue;
            };
            if count_message_marks(slot) > 0 {
                continue;
            }
            let marked = mark_message(slot, &control, openai_dialect);
            if count_message_marks(&marked) > 0 {
                used_blocks = used_blocks.saturating_add(1);
            }
            *slot = marked;
        }
        if limit_reached {
            break;
        }
    }
    if limit_reached {
        warn!(
            limit = MAX_CACHE_CONTROL_BLOCKS,
            "reached the provider cache breakpoint limit; skipping further injection"
        );
    }
    processed
}
