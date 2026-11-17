use rand::distr::Distribution as _;
use rand::rngs::StdRng;
use rand_distr::Beta;

use crate::bandit_math::BanditCell;
use crate::bandit_math::error::BanditMathError;

#[derive(Debug)]
pub struct ScriptedSampler {
    pub samples: Vec<f64>,
    pub position: usize,
}

#[derive(Debug)]
pub enum QualitySampler {
    // Box both payloads so the variants stay equal-sized; variant_size_differences denies a gap.
    Beta(Box<StdRng>),
    Scripted(Box<ScriptedSampler>),
}

impl QualitySampler {
    pub fn sample(&mut self, cell: BanditCell) -> Result<f64, BanditMathError> {
        match self {
            Self::Beta(rng) => {
                let distribution = Beta::new(cell.alpha, cell.beta).map_err(|source| {
                    BanditMathError::Distribution {
                        alpha: cell.alpha,
                        beta: cell.beta,
                        source,
                    }
                })?;
                Ok(distribution.sample(rng.as_mut()))
            }
            Self::Scripted(scripted) => {
                let Some(sample) = scripted.samples.get(scripted.position) else {
                    return Err(BanditMathError::SamplesExhausted);
                };
                if !sample.is_finite() {
                    return Err(BanditMathError::NonFinite {
                        field: "scripted sample",
                    });
                }
                scripted.position = scripted.position.saturating_add(1);
                Ok(*sample)
            }
        }
    }
}
