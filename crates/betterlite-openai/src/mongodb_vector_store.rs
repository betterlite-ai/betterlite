use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

pub mod create;
pub mod decode;
pub mod environment;
pub mod error;
mod params;
pub mod prepare;
mod query;
pub mod search;
mod tests;

pub type Params = BTreeMap<String, Json>;
