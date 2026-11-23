use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct PluginPath {
    pub(crate) plugin_name: String,
}

// list_plugins' enabled_only: bool = False, filled from the words pydantic accepts for a boolean
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PluginListQuery {
    pub(crate) enabled_only: bool,
}
