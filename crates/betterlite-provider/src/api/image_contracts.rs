pub mod edit;
pub mod error;
pub mod generation;
pub mod shared;
mod tests;
pub mod variation;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImageRequest;
pub struct ImageResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImagePrepared;
