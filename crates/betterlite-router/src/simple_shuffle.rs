pub mod error;

use rand::distr::Distribution as _;
use rand::distr::weighted::{self, WeightedIndex};
use rand::{Rng, RngExt as _};

use crate::simple_shuffle::error::ShuffleError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub weight: u32,
    pub rpm: u32,
    pub tpm: u32,
}

#[derive(Clone, Copy)]
enum Metric {
    Weight,
    Rpm,
    Tpm,
}

// LiteLLM tries weight, then rpm, then tpm, and only then picks uniformly
const METRICS: [Metric; 3] = [Metric::Weight, Metric::Rpm, Metric::Tpm];

pub fn pick<R>(candidates: &[Candidate], random: &mut R) -> Result<usize, ShuffleError>
where
    R: Rng,
{
    if candidates.is_empty() {
        return Err(ShuffleError::NoCandidates);
    }
    for metric in METRICS {
        let values = candidates.iter().map(|candidate| match metric {
            Metric::Weight => u64::from(candidate.weight),
            Metric::Rpm => u64::from(candidate.rpm),
            Metric::Tpm => u64::from(candidate.tpm),
        });
        match WeightedIndex::new(values) {
            Ok(index) => return Ok(index.sample(random)),
            Err(weighted::Error::InsufficientNonZero) => {}
            Err(source) => return Err(ShuffleError::Weights { source }),
        }
    }
    Ok(random.random_range(0..candidates.len()))
}

mod tests;
