use rand_distr::BetaError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum BanditMathError {
    #[error("the adaptive bandit cannot use a non-finite {field}")]
    NonFinite { field: &'static str },
    #[error("quality_tier={tier} is not supported; valid tiers are [1, 2, 3]")]
    UnsupportedTier { tier: u8 },
    #[error("pick_best called with no models")]
    NoModels,
    #[error("the request has no scripted bandit sample left")]
    SamplesExhausted,
    #[error("the adaptive bandit cannot draw from Beta({alpha}, {beta})")]
    Distribution {
        alpha: f64,
        beta: f64,
        #[source]
        source: BetaError,
    },
}
