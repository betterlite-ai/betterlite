use crate::helper::HelperResult;

pub struct Group {
    pub invoke: fn(&str, &serde_json::Value) -> Option<HelperResult>,
    pub check: fn(&str, &serde_json::Value) -> Option<Result<(), String>>,
}
