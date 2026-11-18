#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

// numpy's add.reduce keeps eight lanes up to 128 values and halves a longer input before recursing
const PAIRWISE_BLOCK_LIMIT: usize = 128;

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

// np.linalg.norm of a one-dimensional vector is sqrt(x.dot(x)), and the pinned BLAS fuses each product into the sum
pub fn vector_norm(vector: &[f64], index: usize) -> Result<f64, MathError> {
    for value in vector {
        if !value.is_finite() {
            return Err(MathError::NonFinite { index });
        }
    }
    let mut squares = 0.0_f64;
    for value in vector {
        squares = value.mul_add(*value, squares);
    }
    checked_norm(squares, index)
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
    let document_norm = index_norm(document, index)?;
    let mut dot = 0.0_f64;
    for (query_value, document_value) in query.iter().zip(document) {
        dot = query_value.mul_add(*document_value, dot);
    }
    Ok(dot / (query_norm * document_norm))
}

// the index side of similarity_matrix: np.linalg.norm(matrix, axis=1) reduces each row with add.reduce
fn index_norm(vector: &[f64], index: usize) -> Result<f64, MathError> {
    for value in vector {
        if !value.is_finite() {
            return Err(MathError::NonFinite { index });
        }
    }
    checked_norm(pairwise_square_sum(vector), index)
}

fn checked_norm(squares: f64, index: usize) -> Result<f64, MathError> {
    let norm = squares.sqrt();
    if !norm.is_finite() {
        return Err(MathError::NonFinite { index });
    }
    if norm <= 0.0_f64 {
        return Err(MathError::ZeroNorm { index });
    }
    Ok(norm)
}

// pairwise_sum_DOUBLE's three shapes in numpy's loops_utils.h, over the squares the norm reduces
fn pairwise_square_sum(values: &[f64]) -> f64 {
    if values.len() < 8 {
        let mut total = 0.0_f64;
        for value in values {
            total += value * value;
        }
        return total;
    }
    if values.len() <= PAIRWISE_BLOCK_LIMIT {
        return block_square_sum(values);
    }
    let (left, right) = values.split_at((values.len() >> 1) & !7);
    pairwise_square_sum(left) + pairwise_square_sum(right)
}

fn block_square_sum(values: &[f64]) -> f64 {
    // the first chunk seeds the lanes from zero, which is exact for the non-negative squares
    let mut lanes = [0.0_f64; 8];
    let mut chunks = values.chunks_exact(8);
    for chunk in &mut chunks {
        for (lane, value) in lanes.iter_mut().zip(chunk) {
            *lane += value * value;
        }
    }
    let [first, second, third, fourth, fifth, sixth, seventh, eighth] = lanes;
    let mut total = ((first + second) + (third + fourth)) + ((fifth + sixth) + (seventh + eighth));
    for value in chunks.remainder() {
        total += value * value;
    }
    total
}

// numpy's 2-D index rejects a ragged, nonfinite or zero-length batch before any score is computed
pub fn validate_documents(vectors: &[Vec<f64>]) -> Result<(), MathError> {
    let mut width: Option<usize> = None;
    for (index, vector) in vectors.iter().enumerate() {
        index_norm(vector, index)?;
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
