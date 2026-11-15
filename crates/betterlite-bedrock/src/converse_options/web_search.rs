use crate::model_routes::{ParamMap, ParamValue};

// LiteLLM ignores every web_search_options member, so an empty mapping enables grounding by itself.
#[must_use]
pub fn map_web_search_options(model: &str) -> Option<ParamMap> {
    if !model.to_lowercase().contains("nova") {
        tracing::debug!(
            "web_search_options passed but model {model} is not a Nova model. Nova grounding is only supported on Amazon Nova models."
        );
        return None;
    }
    let mut system_tool = ParamMap::new();
    system_tool.insert(
        "name".to_owned(),
        ParamValue::Text("nova_grounding".to_owned()),
    );
    Some(ParamMap::from([(
        "systemTool".to_owned(),
        ParamValue::Mapping(system_tool),
    )]))
}
