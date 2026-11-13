use std::collections::BTreeMap;

use super::split::TextChunk;
use super::wire::AnalyzeResponseItem;

fn span_length(item: &AnalyzeResponseItem) -> usize {
    item.end
        .unwrap_or(0)
        .saturating_sub(item.start.unwrap_or(0))
}

#[must_use]
pub fn merge_chunked_analyze_results(
    text_chunks: &[TextChunk],
    chunk_results: Vec<Vec<AnalyzeResponseItem>>,
) -> Vec<AnalyzeResponseItem> {
    let mut remapped = Vec::new();
    for (chunk, results) in text_chunks.iter().zip(chunk_results) {
        for mut item in results {
            item.start = item
                .start
                .map(|start| start.saturating_add(chunk.char_offset));
            item.end = item.end.map(|end| end.saturating_add(chunk.char_offset));
            remapped.push(item);
        }
    }
    // The stable sort keeps the analyzer's order between equal spans with equal scores.
    remapped.sort_by(|left, right| {
        span_length(right).cmp(&span_length(left)).then_with(|| {
            right
                .score
                .unwrap_or(0.0_f64)
                .total_cmp(&left.score.unwrap_or(0.0_f64))
        })
    });
    let mut merged = Vec::new();
    let mut kept_spans_by_type: BTreeMap<Option<String>, Vec<(usize, usize)>> = BTreeMap::new();
    for item in remapped {
        let (Some(item_start), Some(item_end)) = (item.start, item.end) else {
            merged.push(item);
            continue;
        };
        let kept_spans = kept_spans_by_type
            .entry(item.entity_type.clone())
            .or_default();
        if kept_spans
            .iter()
            .any(|&(kept_start, kept_end)| item_start < kept_end && kept_start < item_end)
        {
            continue;
        }
        kept_spans.push((item_start, item_end));
        merged.push(item);
    }
    merged.sort_by_key(|item| (item.start.unwrap_or(0), item.end.unwrap_or(0)));
    merged
}
