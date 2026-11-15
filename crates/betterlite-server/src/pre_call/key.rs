use serde_json::{Map, Value};

pub trait PreCallKey: Send + Sync {
    fn team_id(&self) -> Option<&str>;
    fn key_aliases(&self) -> Option<&Map<String, Value>>;
    fn key_router_settings(&self) -> Option<&Value>;
}
