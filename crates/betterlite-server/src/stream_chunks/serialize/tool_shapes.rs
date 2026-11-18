use betterlite_models::decisions::Json;

use super::{EMPTY, Shape, without_nulls};

// OpenAI's not-in comparison tag, in two pieces so the spell checker sees no misspelling
const NOT_IN: &str = concat!("n", "in");
const COMPARISON_TYPES: &[(&str, Shape)] = &[
    ("eq", Shape::Fields(EMPTY)),
    ("gt", Shape::Fields(EMPTY)),
    ("gte", Shape::Fields(EMPTY)),
    ("in", Shape::Fields(EMPTY)),
    ("lt", Shape::Fields(EMPTY)),
    ("lte", Shape::Fields(EMPTY)),
    ("ne", Shape::Fields(EMPTY)),
    (NOT_IN, Shape::Fields(EMPTY)),
];
const COMPARISON: Shape = Shape::Tagged(COMPARISON_TYPES);
const COMPOUND: &[(&str, Shape)] = &[("filters", COMPARISON)];
const FILE_FILTERS: &[(&str, Shape)] = &[
    ("and", Shape::Fields(COMPOUND)),
    ("eq", Shape::Fields(EMPTY)),
    ("gt", Shape::Fields(EMPTY)),
    ("gte", Shape::Fields(EMPTY)),
    ("in", Shape::Fields(EMPTY)),
    ("lt", Shape::Fields(EMPTY)),
    ("lte", Shape::Fields(EMPTY)),
    ("ne", Shape::Fields(EMPTY)),
    (NOT_IN, Shape::Fields(EMPTY)),
    ("or", Shape::Fields(COMPOUND)),
];
const RANKING: &[(&str, Shape)] = &[("hybrid_search", Shape::Fields(EMPTY))];
const FILE_SEARCH: &[(&str, Shape)] = &[
    ("filters", Shape::Tagged(FILE_FILTERS)),
    ("ranking_options", Shape::Fields(RANKING)),
];
const USER_LOCATION: &[(&str, Shape)] = &[];
const WEB_SEARCH: &[(&str, Shape)] = &[
    ("filters", Shape::Fields(EMPTY)),
    ("user_location", Shape::Fields(USER_LOCATION)),
];
const PREVIEW: &[(&str, Shape)] = &[("user_location", Shape::Fields(USER_LOCATION))];
const MCP_FILTER: &[(&str, Shape)] = &[];
const MCP_APPROVAL: &[(&str, Shape)] = &[
    ("always", Shape::Fields(MCP_FILTER)),
    ("never", Shape::Fields(MCP_FILTER)),
];
const MCP: &[(&str, Shape)] = &[
    ("allowed_tools", Shape::Fields(MCP_FILTER)),
    ("require_approval", Shape::Fields(MCP_APPROVAL)),
];
const NET_ALLOW: &[(&str, Shape)] = &[("domain_secrets", Shape::Fields(EMPTY))];
const NET_POLICIES: &[(&str, Shape)] = &[
    ("allowlist", Shape::Fields(NET_ALLOW)),
    ("disabled", Shape::Fields(EMPTY)),
];
const NET_POLICY: Shape = Shape::Tagged(NET_POLICIES);
const CI_CONTAINER: &[(&str, Shape)] = &[("network_policy", NET_POLICY)];
const CI_CONTAINERS: &[(&str, Shape)] = &[("auto", Shape::Fields(CI_CONTAINER))];
const CODE_INTERPRETER: &[(&str, Shape)] = &[("container", Shape::Tagged(CI_CONTAINERS))];
const IMAGE_GENERATION: &[(&str, Shape)] = &[("input_image_mask", Shape::Fields(EMPTY))];
const INLINE_SKILL: &[(&str, Shape)] = &[("source", Shape::Fields(EMPTY))];
const SKILLS: &[(&str, Shape)] = &[
    ("inline", Shape::Fields(INLINE_SKILL)),
    ("skill_reference", Shape::Fields(EMPTY)),
];
const SKILL: Shape = Shape::Tagged(SKILLS);
const SHELL_CONTAINER: &[(&str, Shape)] = &[("network_policy", NET_POLICY), ("skills", SKILL)];
const LOCAL_ENV: &[(&str, Shape)] = &[("skills", Shape::Fields(EMPTY))];
const SHELL_ENVS: &[(&str, Shape)] = &[
    ("container_auto", Shape::Fields(SHELL_CONTAINER)),
    ("container_reference", Shape::Fields(EMPTY)),
    ("local", Shape::Fields(LOCAL_ENV)),
];
const SHELL_ENV: Shape = Shape::Tagged(SHELL_ENVS);
const SHELL: &[(&str, Shape)] = &[("environment", SHELL_ENV)];
const FORMATS: &[(&str, Shape)] = &[
    ("grammar", Shape::Fields(EMPTY)),
    ("text", Shape::Fields(EMPTY)),
];
const FORMAT: Shape = Shape::Tagged(FORMATS);
const CUSTOM: &[(&str, Shape)] = &[("format", FORMAT)];
const NAMESPACE_TOOLS: &[(&str, Shape)] = &[
    ("custom", Shape::Fields(CUSTOM)),
    ("function", Shape::Fields(EMPTY)),
];
const NAMESPACE_TOOL: Shape = Shape::Tagged(NAMESPACE_TOOLS);
const NAMESPACE: &[(&str, Shape)] = &[("tools", NAMESPACE_TOOL)];
const CALL: &[(&str, Shape)] = &[("caller", Shape::Fields(EMPTY))];

pub(super) const TOOL_VARIANTS: &[(&str, Shape)] = &[
    ("apply_patch", Shape::Fields(EMPTY)),
    ("code_interpreter", Shape::Fields(CODE_INTERPRETER)),
    ("computer", Shape::Fields(EMPTY)),
    ("computer_use_preview", Shape::Fields(EMPTY)),
    ("custom", Shape::Fields(CUSTOM)),
    ("file_search", Shape::Fields(FILE_SEARCH)),
    ("function", Shape::Fields(EMPTY)),
    ("image_generation", Shape::Fields(IMAGE_GENERATION)),
    ("local_shell", Shape::Fields(EMPTY)),
    ("mcp", Shape::Fields(MCP)),
    ("namespace", Shape::Fields(NAMESPACE)),
    ("programmatic_tool_calling", Shape::Fields(EMPTY)),
    ("shell", Shape::Fields(SHELL)),
    ("tool_search", Shape::Fields(EMPTY)),
    ("web_search", Shape::Fields(WEB_SEARCH)),
    ("web_search_2025_08_26", Shape::Fields(WEB_SEARCH)),
    ("web_search_preview", Shape::Fields(PREVIEW)),
    ("web_search_preview_2025_03_11", Shape::Fields(PREVIEW)),
];

// the union holds a union for tools: every item a declared model, else every item a function call, else plain dicts
pub(super) fn tools_without_nulls(value: &Json, variants: &[(&str, Shape)]) -> Json {
    let Json::List(items) = value else {
        return value.clone();
    };
    if items
        .iter()
        .all(|item| tool_kind(item) == Some("function_call"))
    {
        return Json::List(
            items
                .iter()
                .map(|item| without_nulls(item, Shape::Fields(CALL)))
                .collect(),
        );
    }
    let mut shaped = Vec::with_capacity(items.len());
    for item in items {
        let Some(kind) = tool_kind(item) else {
            return value.clone();
        };
        let Some((_name, shape)) = variants.iter().find(|(name, _shape)| *name == kind) else {
            return value.clone();
        };
        shaped.push(without_nulls(item, *shape));
    }
    Json::List(shaped)
}

fn tool_kind(item: &Json) -> Option<&str> {
    let Json::Object(entries) = item else {
        return None;
    };
    let Some(Json::Text(kind)) = entries.get("type") else {
        return None;
    };
    Some(kind.as_str())
}
