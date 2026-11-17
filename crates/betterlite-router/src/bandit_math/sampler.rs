use rand::SeedableRng as _;
use rand::distr::Distribution as _;
use rand::rngs::StdRng;
use rand_distr::Beta;

use crate::bandit_math::BanditCell;
use crate::bandit_math::error::BanditMathError;

#[derive(Debug, Clone)]
pub enum QualitySampler {
    Beta(StdRng),
    Scripted(ScriptedSampler),
}

impl QualitySampler {
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self::Beta(StdRng::seed_from_u64(seed))
    }

    pub fn sample(&mut self, cell: BanditCell) -> Result<f64, BanditMathError> {
        match self {
            Self::Beta(rng) => {
                let distribution: Beta<f64> =
                    Beta::new(cell.alpha, cell.beta).map_err(|source| {
                        BanditMathError::Distribution {
                            alpha: cell.alpha,
                            beta: cell.beta,
                            source,
                        }
                    })?;
                Ok(distribution.sample(rng))
            }
            Self::Scripted(scripted) => scripted.next_sample(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScriptedSampler {
    samples: Vec<f64>,
    next: usize,
}

impl ScriptedSampler {
    #[must_use]
    pub const fn new(samples: Vec<f64>) -> Self {
        Self { samples, next: 0 }
    }

    fn next_sample(&mut self) -> Result<f64, BanditMathError> {
        let Some(sample) = self.samples.get(self.next) else {
            return Err(BanditMathError::SamplesExhausted);
        };
        if !sample.is_finite() {
            return Err(BanditMathError::NonFinite {
                field: "scripted sample",
            });
        }
        self.next = self.next.saturating_add(1);
        Ok(*sample)
    }
}
