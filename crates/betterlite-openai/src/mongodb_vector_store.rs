use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

pub mod create;
pub mod decode;
pub mod environment;
pub mod error;
pub mod prepare;
pub mod search;
mod tests;

pub type Params = BTreeMap<String, Json>;
