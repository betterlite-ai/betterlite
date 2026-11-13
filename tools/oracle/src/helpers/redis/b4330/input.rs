use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) namespace: Option<String>,
    pub(crate) default_ttl: i64,
    #[serde(default)]
    pub(crate) cluster: bool,
    #[serde(default)]
    pub(crate) refused: bool,
    #[serde(default)]
    pub(crate) replies: BTreeMap<String, Vec<Reply>>,
    pub(crate) steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reply {
    Bulk(String),
    Simple(String),
    Int(i64),
    Error(String),
    Nil,
    Array(Vec<Self>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Step {
    Declare(Operation),
    Await(usize),
    AwaitAll,
    Flush,
    Missing(String),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    Mget {
        keys: Vec<String>,
    },
    Script {
        source: String,
        keys: Vec<String>,
        #[serde(default)]
        args: Vec<Arg>,
    },
    Increment {
        key: String,
        value: f64,
        #[serde(default)]
        ttl: Option<i64>,
    },
    Set {
        key: String,
        value: Value,
        #[serde(default)]
        ttl: Option<f64>,
    },
    Delete {
        key: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Arg {
    Text(String),
    Int(i64),
    Float(f64),
}
