use crate::model_routes::{ParamMap, ParamValue};

const NOVA_MARKER: &str = "nova";
const SYSTEM_TOOL_NAME: &str = "nova_grounding";

// LiteLLM ignores every web_search_options member, so an empty mapping enables grounding by itself.
#[must_use]
pub fn map_web_search_options(model: &str) -> Option<ParamMap> {
    if !model.to_lowercase().contains(NOVA_MARKER) {
        tracing::debug!(
            "web_search_options passed but model {model} is not a Nova model. Nova grounding is only supported on Amazon Nova models."
        );
        return None;
    }
    let mut system_tool = ParamMap::new();
    system_tool.insert(
        "name".to_owned(),
        ParamValue::Text(SYSTEM_TOOL_NAME.to_owned()),
    );
    Some(ParamMap::from([(
        "systemTool".to_owned(),
        ParamValue::Mapping(system_tool),
    )]))
}
