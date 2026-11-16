use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const ITEM_ADDED: &str = "response.output_item.added";
const ITEM_DONE: &str = "response.output_item.done";

// one map per request: the id an output index gets in output_item.added is the id every later event of that index carries
#[derive(Debug, Default)]
pub struct StreamItemIds {
    anchors: BTreeMap<i128, String>,
}

impl StreamItemIds {
    pub fn normalize(&mut self, mut chunk: IndexMap<String, Json>) -> IndexMap<String, Json> {
        let Some(index) = output_index(chunk.get("output_index")) else {
            return chunk;
        };
        let is_added = matches!(chunk.get("type"), Some(Json::Text(kind)) if kind == ITEM_ADDED);
        if is_added {
            if let Some(Json::Object(item)) = chunk.get("item")
                && let Some(Json::Text(id)) = item.get("id")
            {
                self.anchors.insert(index, id.clone());
            }
            return chunk;
        }
        let Some(anchor) = self.anchors.get(&index) else {
            return chunk;
        };
        let stable = Json::Text(anchor.clone());
        if matches!(chunk.get("item_id"), Some(Json::Text(_))) {
            chunk.insert(String::from("item_id"), stable);
            return chunk;
        }
        let is_done = matches!(chunk.get("type"), Some(Json::Text(kind)) if kind == ITEM_DONE);
        if is_done && let Some(Json::Object(item)) = chunk.get_mut("item") {
            item.insert(String::from("id"), stable);
        }
        chunk
    }
}

// Python's isinstance(value, int) also accepts a bool, and True and 1 are the same dictionary key
fn output_index(value: Option<&Json>) -> Option<i128> {
    match value? {
        Json::Bool(flag) => Some(i128::from(*flag)),
        Json::Number(number) => number.to_string().parse::<i128>().ok(),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
