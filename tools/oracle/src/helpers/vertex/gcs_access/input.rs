use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ConfiguredPrefix,
    EncodedTraversal,
    ForeignBucket,
    TerminalSlash,
    PercentReencoding,
    TrustedLegacy,
    ForgedTrust,
    FilenameSanitization,
    ModelPath,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) params: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) trusted: Vec<(String, WireValue)>,
    // a plain JSON mapping never becomes the trusted snapshot, so this input only has to decode
    #[serde(rename = "plain", default)]
    pub(crate) _plain: Vec<(String, WireValue)>,
    #[serde(default)]
    pub(crate) file_ids: Vec<String>,
    #[serde(default)]
    pub(crate) filenames: Vec<Option<String>>,
    #[serde(default)]
    pub(crate) model_paths: Vec<Option<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum WireValue {
    Text(String),
    Boolean(bool),
    Null(()),
    Other(IgnoredAny),
}
