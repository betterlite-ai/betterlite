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

// one scored row with its group threshold, so grouping never looks a route up again
#[derive(Debug, Clone, Copy)]
struct Scored<'a> {
    route: &'a str,
    score: f64,
    threshold: f64,
    index: usize,
}

// one route's standing: the threshold of its first route and the mean of its rows inside the top_k window
#[derive(Debug, Clone, Copy)]
struct RouteScore<'a> {
    route: &'a str,
    threshold: f64,
    mean: f64,
}

pub fn select(entries: &[BuiltEntry], query: &[f64]) -> Result<Option<String>, MathError> {
    if entries.is_empty() {
        return Ok(None);
    }
    for route_score in route_scores(top_scores(entries, query)?) {
        // the source reads a zero threshold as falsy, which always passes; a nonzero one compares normally
        let comparable = route_score.threshold > 0.0 || route_score.threshold < 0.0;
        if !comparable || route_score.mean >= route_score.threshold {
            return Ok(Some(route_score.route.to_owned()));
        }
    }
    Ok(None)
}

fn top_scores<'a>(entries: &'a [BuiltEntry], query: &[f64]) -> Result<Vec<Scored<'a>>, MathError> {
    let mut scored: Vec<Scored<'a>> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let score = cosine_similarity(query, &entry.vector, index)?;
        scored.push(Scored {
            route: entry.route.as_str(),
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

fn route_scores(scored: Vec<Scored<'_>>) -> Vec<RouteScore<'_>> {
    let mut grouped: Vec<(&str, f64, Vec<f64>)> = Vec::new();
    for row in scored {
        match grouped.iter_mut().find(|(name, _, _)| *name == row.route) {
            Some((_, _, scores)) => scores.push(row.score),
            None => grouped.push((row.route, row.threshold, vec![row.score])),
        }
    }
    let mut totals: Vec<RouteScore<'_>> = Vec::with_capacity(grouped.len());
    for (route, threshold, scores) in grouped {
        totals.push(RouteScore {
            route,
            threshold,
            mean: mean(&scores),
        });
    }
    // a stable descending sort keeps the first-appearance order of equal means
    totals.sort_by(|left, right| right.mean.total_cmp(&left.mean));
    totals
}
