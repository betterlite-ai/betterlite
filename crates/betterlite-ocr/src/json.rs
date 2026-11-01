use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub type Object = IndexMap<String, Json>;

// IndexMap keeps insertion order without the workspace-wide preserve_order feature this crate skips.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Object(Object),
}
