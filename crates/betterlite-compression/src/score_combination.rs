pub const DEFAULT_BM25_WEIGHT: f64 = 0.4;

pub fn combine_scores(bm25_scores: &[f64], embedding_scores: &[f64], bm25_weight: f64) -> Vec<f64> {
    let normalized_bm25 = normalize_scores(bm25_scores);
    let normalized_embedding = normalize_scores(embedding_scores);
    let embedding_weight = bm25_weight.mul_add(-1.0, 1.0);
    normalized_bm25
        .iter()
        .zip(&normalized_embedding)
        .map(|(bm25, embedding)| {
            let bm25_part = bm25_weight.mul_add(*bm25, 0.0);
            let embedding_part = embedding_weight.mul_add(*embedding, 0.0);
            bm25_part.mul_add(1.0, embedding_part)
        })
        .collect()
}

fn normalize_scores(scores: &[f64]) -> Vec<f64> {
    let lowest = scores.iter().copied().reduce(f64::min).unwrap_or(0.0);
    let highest = scores.iter().copied().reduce(f64::max).unwrap_or(0.0);
    let range = lowest.mul_add(-1.0, highest);
    if range <= 0.0 {
        return vec![0.0; scores.len()];
    }
    scores
        .iter()
        .map(|value| lowest.mul_add(-1.0, *value).mul_add(range.recip(), 0.0))
        .collect()
}

mod tests;
