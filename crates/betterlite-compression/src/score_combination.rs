use std::ops::{Add, Div, Mul, Sub};

pub const DEFAULT_BM25_WEIGHT: f64 = 0.4;

pub fn combine_scores(bm25_scores: &[f64], embedding_scores: &[f64], bm25_weight: f64) -> Vec<f64> {
    let normalized_bm25 = normalize_scores(bm25_scores);
    let normalized_embedding = normalize_scores(embedding_scores);
    let embedding_weight = Sub::sub(1.0_f64, bm25_weight);
    normalized_bm25
        .iter()
        .zip(&normalized_embedding)
        .map(|(bm25, embedding)| {
            Add::add(
                Mul::mul(bm25_weight, *bm25),
                Mul::mul(embedding_weight, *embedding),
            )
        })
        .collect()
}

fn normalize_scores(scores: &[f64]) -> Vec<f64> {
    let lowest = scores.iter().copied().reduce(f64::min).unwrap_or(0.0);
    let highest = scores.iter().copied().reduce(f64::max).unwrap_or(0.0);
    let range = Sub::sub(highest, lowest);
    if range <= 0.0 {
        return vec![0.0; scores.len()];
    }
    scores
        .iter()
        .map(|value| Div::div(Sub::sub(*value, lowest), range))
        .collect()
}

mod tests;
