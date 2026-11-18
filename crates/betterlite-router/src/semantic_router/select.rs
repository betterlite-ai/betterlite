use std::sync::Arc;

use crate::semantic_math::{MathError, cosine_similarity, mean};

// SemanticRouter's top_k default: how many utterances reach route scoring
pub const TOP_K: usize = 5;

// a routed row with its group threshold, so scoring never looks a route up again
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltEntry {
    pub route: Arc<str>,
    pub threshold: f64,
    pub vector: Vec<f64>,
}

// one scored row with its group threshold, so grouping never looks a route up again
#[derive(Debug, Clone, Copy)]
struct Scored<'row> {
    route: &'row str,
    score: f64,
    threshold: f64,
    index: usize,
}

// one route's standing: the threshold of its first route and the mean of its rows inside the top_k window
#[derive(Debug, Clone, Copy)]
struct RouteScore<'row> {
    route: &'row str,
    threshold: f64,
    mean: f64,
}

pub fn select(entries: &[BuiltEntry], query: &[f64]) -> Result<Option<String>, MathError> {
    if entries.is_empty() {
        return Ok(None);
    }
    let scored = top_scores(entries, query)?;
    for route_score in route_scores(&scored) {
        // the source reads a zero threshold as falsy, which always passes; a nonzero one compares normally
        let comparable = route_score.threshold != 0.0_f64;
        if !comparable || route_score.mean >= route_score.threshold {
            return Ok(Some(route_score.route.to_owned()));
        }
    }
    Ok(None)
}

fn top_scores<'row>(
    entries: &'row [BuiltEntry],
    query: &[f64],
) -> Result<Vec<Scored<'row>>, MathError> {
    let mut scored: Vec<Scored<'row>> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let score = cosine_similarity(query, &entry.vector, index)?;
        scored.push(Scored {
            route: &entry.route,
            score,
            threshold: entry.threshold,
            index,
        });
    }
    // the highest top_k scores, the earlier row winning a tie, then the ascending order the source groups by
    scored.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.index.cmp(&right.index))
    });
    scored.truncate(TOP_K.min(scored.len()));
    scored.sort_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(scored)
}

fn route_scores<'row>(scored: &[Scored<'row>]) -> Vec<RouteScore<'row>> {
    // the threshold of the first route of each name, in the order the group first appears
    let mut routes: Vec<(&'row str, f64)> = Vec::new();
    for row in scored {
        if !routes.iter().any(|(name, _)| *name == row.route) {
            routes.push((row.route, row.threshold));
        }
    }
    let mut totals: Vec<RouteScore<'_>> = Vec::with_capacity(routes.len());
    for (route, threshold) in routes {
        let group_scores: Vec<f64> = scored
            .iter()
            .filter(|row| row.route == route)
            .map(|row| row.score)
            .collect();
        totals.push(RouteScore {
            route,
            threshold,
            mean: mean(&group_scores),
        });
    }
    // a stable descending sort keeps the first-appearance order of equal means
    totals.sort_by(|left, right| right.mean.total_cmp(&left.mean));
    totals
}
