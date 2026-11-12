use crate::resolution::ResolutionError;

#[derive(Debug, thiserror::Error)]
pub enum RegionError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
}
