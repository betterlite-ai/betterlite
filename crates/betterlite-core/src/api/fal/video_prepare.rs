pub mod create;
pub mod decode;
pub mod duration;
pub mod error;
pub mod params;
pub mod paths;
pub mod profile;
pub mod settings;
pub mod size;
mod tests;
pub mod urls;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub type VideoHeaders = BTreeMap<String, String>;
pub type VideoParams = IndexMap<String, Json>;
