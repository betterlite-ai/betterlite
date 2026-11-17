#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MathError {
    #[error("vector {index} has {found} dimensions, not the expected {expected}")]
    Dimension {
        index: usize,
        expected: usize,
        found: usize,
    },
    #[error("vector {index} holds a value or square that is not finite")]
    NonFinite { index: usize },
    #[error("vector {index} has no magnitude, so its similarity is undefined")]
    ZeroNorm { index: usize },
}

pub fn vector_norm(vector: &[f64], index: usize) -> Result<f64, MathError> {
    let mut squares = 0.0_f64;
    for value in vector {
        if !value.is_finite() {
            return Err(MathError::NonFinite { index });
        }
        squares += value * value;
    }
    let norm = squares.sqrt();
    if !norm.is_finite() {
        return Err(MathError::NonFinite { index });
    }
    if norm <= 0.0_f64 {
        return Err(MathError::ZeroNorm { index });
    }
    Ok(norm)
}

pub fn cosine_similarity(query: &[f64], document: &[f64], index: usize) -> Result<f64, MathError> {
    if query.len() != document.len() {
        return Err(MathError::Dimension {
            index,
            expected: query.len(),
            found: document.len(),
        });
    }
    let query_norm = vector_norm(query, 0)?;
    let document_norm = vector_norm(document, index)?;
    let mut dot = 0.0_f64;
    for (query_value, document_value) in query.iter().zip(document) {
        dot += query_value * document_value;
    }
    Ok(dot / (query_norm * document_norm))
}

// numpy's 2-D index rejects a ragged, nonfinite or zero-length batch before any score is computed
pub fn validate_documents(vectors: &[Vec<f64>]) -> Result<(), MathError> {
    let mut width: Option<usize> = None;
    for (index, vector) in vectors.iter().enumerate() {
        vector_norm(vector, index)?;
        match width {
            Some(expected) if expected != vector.len() => {
                return Err(MathError::Dimension {
                    index,
                    expected,
                    found: vector.len(),
                });
            }
            Some(_) => {}
            None => width = Some(vector.len()),
        }
    }
    Ok(())
}

// the mean of an empty group is NaN, the float division of zero by zero that never reaches a real route
#[must_use]
pub fn mean(values: &[f64]) -> f64 {
    let mut total = 0.0_f64;
    for value in values {
        total += value;
    }
    let count = f64::from(u32::try_from(values.len()).unwrap_or(u32::MAX));
    total / count
}
