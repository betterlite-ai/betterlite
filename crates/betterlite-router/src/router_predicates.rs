use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub mod kinds;
pub mod lists;
mod map_values;
pub mod mock_testing;
pub mod params;
pub mod source_text;
pub mod success_key;
mod tests;

pub type JsonMap = IndexMap<String, Json>;
