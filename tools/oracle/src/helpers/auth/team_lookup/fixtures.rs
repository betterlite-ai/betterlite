mod fakes;
mod input;
mod wiring;

pub(crate) use fakes::Levels;
pub(crate) use input::{Flag, Function, Input};
pub(crate) use wiring::{begin, wire};
