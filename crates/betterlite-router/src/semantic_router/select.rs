use crate::semantic_math::{MathError, cosine_similarity, mean};

// SemanticRouter's top_k default: how many utterances reach route scoring
pub const TOP_K: usize = 5;

// a routed row with its group threshold, so scoring never looks a route up again
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltEntry {
    pub route: String,
    pub threshold: f64,
    pub vector: Vec<f64>,
}

pub fn select(entries: &[BuiltEntry], query: &[f64]) -> Result<Option<String>, MathError> {
    if entries.is_empty() {
        return Ok(None);
    }
    for (route, threshold, total) in route_totals(top_scores(entries, query)?) {
        // the source reads a zero threshold as falsy, which always passes; a nonzero one compares normally
        let comparable = threshold > 0.0 || threshold < 0.0;
        if !comparable || total >= threshold {
            return Ok(Some(route.to_owned()));
        }
    }
    Ok(None)
}

fn top_scores<'a>(
    entries: &'a [BuiltEntry],
    query: &[f64],
) -> Result<Vec<(&'a str, f64, f64, usize)>, MathError> {
    let mut scored: Vec<(&str, f64, f64, usize)> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let score = cosine_similarity(query, &entry.vector, index)?;
        scored.push((entry.route.as_str(), score, entry.threshold, index));
    }
    // the highest top_k scores, the earlier row winning a tie, then the ascending order the source groups by
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.3.cmp(&right.3))
    });
    scored.truncate(TOP_K.min(scored.len()));
    scored.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.3.cmp(&right.3))
    });
    Ok(scored)
}

fn route_totals(scored: Vec<(&str, f64, f64, usize)>) -> Vec<(&str, f64, f64)> {
    let mut grouped: Vec<(&str, f64, Vec<f64>)> = Vec::new();
    for (route, score, threshold, _index) in scored {
        match grouped.iter_mut().find(|(name, _, _)| *name == route) {
            Some((_, _, scores)) => scores.push(score),
            None => grouped.push((route, threshold, vec![score])),
        }
    }
    let mut totals: Vec<(&str, f64, f64)> = Vec::with_capacity(grouped.len());
    for (route, threshold, scores) in &grouped {
        totals.push((*route, *threshold, mean(scores)));
    }
    // a stable descending sort keeps the first-appearance order of equal totals
    totals.sort_by(|left, right| right.2.total_cmp(&left.2));
    totals
}
