use super::AnalyzerSettings;
use super::wire::AnalyzeResponseItem;

#[must_use]
pub(super) fn filter_by_score(
    settings: &AnalyzerSettings,
    analyze_results: Vec<AnalyzeResponseItem>,
) -> Vec<AnalyzeResponseItem> {
    let thresholds = &settings.score_thresholds;
    let deny_list = &settings.entities_deny_list;
    if thresholds.is_empty() && deny_list.is_empty() {
        return analyze_results;
    }
    let mut filtered = Vec::new();
    for item in analyze_results {
        if item
            .entity_type
            .as_ref()
            .is_some_and(|entity_type| deny_list.contains(entity_type))
        {
            continue;
        }
        let threshold = item
            .entity_type
            .as_ref()
            .and_then(|entity_type| thresholds.get(entity_type))
            .or_else(|| thresholds.get("ALL"));
        if threshold.is_some_and(|minimum| item.score.is_none_or(|score| score < *minimum)) {
            continue;
        }
        filtered.push(item);
    }
    filtered
}
