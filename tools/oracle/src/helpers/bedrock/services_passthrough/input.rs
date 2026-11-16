use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

// The Rust helper never sees the case env, so the environment a run depends on travels in the input
// next to the same values the case sets for the Python subprocess.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Environment {
    #[serde(default)]
    pub(super) region_name: Option<String>,
    #[serde(default)]
    pub(super) runtime_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum Input {
    #[serde(rename = "url")]
    Url {
        #[serde(default)]
        environment: Environment,
        runs: Vec<UrlRun>,
    },
    #[serde(rename = "sign")]
    Sign {
        #[serde(default)]
        bearer: Option<String>,
        runs: Vec<SignRun>,
    },
    #[serde(rename = "reply")]
    Reply { runs: Vec<ReplyRun> },
    #[serde(rename = "collector")]
    Collector {
        model: String,
        endpoint: String,
        split: bool,
        frames: Vec<Frame>,
    },
    #[serde(rename = "selection")]
    Selection {
        runs: Vec<SelectionRun>,
        frames: Vec<Frame>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlRun {
    pub(super) model: String,
    pub(super) endpoint: String,
    #[serde(default)]
    pub(super) region: Option<String>,
    #[serde(default)]
    pub(super) model_id: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) runtime_endpoint: Option<String>,
    #[serde(default)]
    pub(super) query: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SignRun {
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) data: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplyRun {
    pub(super) provider: String,
    pub(super) endpoint: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SelectionRun {
    pub(super) model: String,
    pub(super) endpoint: String,
}

// One AWS event-stream message, before the frame's CRC32 bytes are added.
#[derive(Debug, Deserialize)]
pub(crate) struct Frame {
    pub(super) event_type: String,
    pub(super) payload: String,
}
