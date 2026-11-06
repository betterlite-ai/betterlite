use std::collections::BTreeMap;

use betterlite_bedrock::event_frames::beta::BetaHeader;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Frames(Frames),
    Stream(Stream),
    Beta(Beta),
    Sse(Sse),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frames {
    pub(crate) chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stream {
    pub(crate) chunks: Vec<String>,
    pub(crate) response_headers: Option<BTreeMap<String, String>>,
    pub(crate) shape_loaded: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Beta {
    pub(crate) headers: BTreeMap<String, BetaHeader>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sse {
    pub(crate) event: Option<String>,
    pub(crate) data: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) retry: Option<i64>,
}
