use thiserror::Error;

#[derive(Debug, Error)]
pub enum RandomError {
    #[error("the operating system could not provide secure randomness: {0}")]
    Unavailable(#[from] getrandom::Error),
}

pub trait Entropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), RandomError>;
}

pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), RandomError> {
        getrandom::fill(bytes).map_err(RandomError::Unavailable)
    }
}
