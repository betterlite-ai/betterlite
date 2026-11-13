use rand::distr::weighted;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShuffleError {
    #[error("there are no deployments to choose from")]
    NoCandidates,
    #[error("the deployment weights cannot be sampled: {source}")]
    Weights {
        #[source]
        source: weighted::Error,
    },
}
