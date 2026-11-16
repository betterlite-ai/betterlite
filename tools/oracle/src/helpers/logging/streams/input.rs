use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Probe {
    Chat {
        responses: Vec<Value>,
    },
    Stream {
        chunk_lists: Vec<Vec<Value>>,
    },
    Overlay {
        response: Value,
        #[serde(default)]
        texts: Option<Vec<Option<String>>>,
    },
}
