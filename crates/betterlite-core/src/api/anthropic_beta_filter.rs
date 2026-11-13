pub mod error;
pub mod matrix_filter;
pub mod native_injection;
mod tests;

use indexmap::IndexMap;

pub type Headers = IndexMap<String, String>;
