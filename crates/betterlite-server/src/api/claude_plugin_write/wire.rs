use betterlite_core::api::claude_plugin_policy::source::PluginSpec;
use serde::Deserialize;

// RegisterPluginRequest extends PluginSpec with the plugin name, as the pydantic model does
#[derive(Debug, Deserialize)]
pub(crate) struct CreatePluginBody {
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) spec: PluginSpec,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PluginPath {
    pub(crate) plugin_name: String,
}
