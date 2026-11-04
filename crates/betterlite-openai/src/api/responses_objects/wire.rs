use crate::api::responses_shapes::wire::{ShapeDecodeError, WireDecodeLimits as ShapeLimits};
use thiserror::Error;

pub type WireDecodeLimits = ShapeLimits;

#[derive(Debug, Error)]
pub enum ResponsesObjectsDecodeError {
    #[error(transparent)]
    ResponsesShapes(#[from] ShapeDecodeError),
}
