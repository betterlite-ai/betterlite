#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PluginListQuery {
    // list_plugins' enabled_only: bool = False, filled from the words pydantic accepts for a boolean
    pub(crate) enabled_only: bool,
}
