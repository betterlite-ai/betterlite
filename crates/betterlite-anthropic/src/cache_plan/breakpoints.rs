use std::ops::Range;

use crate::cache_plan::UnsupportedCachePlan;
use crate::cache_plan::decode::{Decoded, Kind, Position};
use crate::cache_plan::wire::Ttl;

const MAX_BREAKPOINTS: usize = 4;
const LOOKBACK_GROUPS: usize = 20;

pub(super) fn markers(decoded: &Decoded) -> Result<Vec<Option<Ttl>>, UnsupportedCachePlan> {
    let mut markers: Vec<Option<Ttl>> = decoded
        .positions
        .iter()
        .map(|position| position.marker)
        .collect();
    let automatic = decoded
        .positions
        .iter()
        .rposition(|position| !matches!(position.kind, Kind::Thinking | Kind::BlankText));
    if let Some(requested) = decoded.cache_control
        && let Some(slot) = automatic.and_then(|index| markers.get_mut(index))
    {
        match *slot {
            Some(existing) if existing != requested => {
                return Err(UnsupportedCachePlan::ConflictingCacheTtl);
            }
            Some(_) => {}
            None => *slot = Some(requested),
        }
    }
    let ttls: Vec<Ttl> = markers.iter().flatten().copied().collect();
    if ttls.len() > MAX_BREAKPOINTS {
        return Err(UnsupportedCachePlan::TooManyCacheBreakpoints);
    }
    if ttls
        .windows(2)
        .any(|pair| matches!(pair, [earlier, later] if earlier.seconds() < later.seconds()))
    {
        return Err(UnsupportedCachePlan::InvalidCacheTtlOrder);
    }
    Ok(markers)
}

// consecutive tool_use blocks, or consecutive tool_result blocks, of one message share a group
pub(super) fn groups(positions: &[Position]) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0_usize;
    for group in positions.chunk_by(|left, right| {
        left.section == right.section
            && left.message_index == right.message_index
            && matches!(left.kind, Kind::ToolUse | Kind::ToolResult)
            && left.kind == right.kind
    }) {
        let end = start.saturating_add(group.len());
        groups.push(start..end);
        start = end;
    }
    groups
}

pub(super) fn lookback<'chain>(
    groups: &[Range<usize>],
    chain: &'chain [String],
    index: usize,
) -> Vec<&'chain str> {
    let end = index.saturating_add(1);
    groups
        .iter()
        .filter(|group| group.start <= index)
        .rev()
        .take(LOOKBACK_GROUPS)
        .flat_map(|group| {
            chain
                .get(group.start..group.end.min(end))
                .unwrap_or_default()
                .iter()
                .rev()
        })
        .map(String::as_str)
        .collect()
}
