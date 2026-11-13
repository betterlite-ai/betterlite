use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

const CODE_EXECUTION_NAME: &str = "litellm_code_execution";
const CODE_EXECUTION_DESCRIPTION: &str = "Execute Python code in a sandboxed environment. Use this to run code that generates files, processes data, or performs computations. Generated files will be returned directly.";
const MAX_DESCRIPTION_CHARS: usize = 1024;
// LiteLLM keeps 1024 characters, so the three dots displace the last three
const KEPT_DESCRIPTION_CHARS: usize = 1021;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Json,
}

#[must_use]
pub fn convert_skill_to_tool(skill: &SkillTable) -> FunctionTool {
    FunctionTool {
        kind: "function".to_owned(),
        function: FunctionDefinition {
            name: skill.skill_id.replace('-', "_").replace(' ', "_"),
            description: tool_description(skill),
            parameters: metadata_parameters(skill),
        },
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn convert_skill_to_anthropic_tool(skill: &SkillTable) -> AnthropicTool {
    AnthropicTool {
        name: skill.skill_id.replace('-', "_").replace(' ', "_"),
        description: tool_description(skill),
        input_schema: metadata_parameters(skill),
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_litellm_code_execution_tool() -> FunctionTool {
    FunctionTool {
        kind: "function".to_owned(),
        function: FunctionDefinition {
            name: CODE_EXECUTION_NAME.to_owned(),
            description: CODE_EXECUTION_DESCRIPTION.to_owned(),
            parameters: code_parameters(),
        },
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_litellm_code_execution_tool_anthropic() -> AnthropicTool {
    AnthropicTool {
        name: CODE_EXECUTION_NAME.to_owned(),
        description: CODE_EXECUTION_DESCRIPTION.to_owned(),
        input_schema: code_parameters(),
    }
}

fn tool_description(skill: &SkillTable) -> String {
    let description = skill
        .instructions
        .clone()
        .or_else(|| skill.description.clone())
        .or_else(|| skill.display_title.clone())
        .unwrap_or_else(|| format!("Skill: {}", skill.skill_id));
    truncate_description(description)
}

fn truncate_description(description: String) -> String {
    if description.chars().count() <= MAX_DESCRIPTION_CHARS {
        return description;
    }
    let kept: String = description.chars().take(KEPT_DESCRIPTION_CHARS).collect();
    format!("{kept}...")
}

fn metadata_parameters(skill: &SkillTable) -> Json {
    let empty_schema = Json::Object(Object::from([
        ("type".to_owned(), Json::Text("object".to_owned())),
        ("properties".to_owned(), Json::Object(Object::new())),
        ("required".to_owned(), Json::List(Vec::new())),
    ]));
    let parameters = skill
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("parameters"));
    let Some(parameters) = parameters else {
        return empty_schema;
    };
    match schema_value(parameters) {
        Json::Object(fields) if !fields.is_empty() => Json::Object(fields),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => empty_schema,
    }
}

fn schema_value(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(schema_value).collect()),
        JsonData::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), schema_value(field)))
                .collect(),
        ),
    }
}

fn code_parameters() -> Json {
    Json::Object(Object::from([
        ("type".to_owned(), Json::Text("object".to_owned())),
        (
            "properties".to_owned(),
            Json::Object(Object::from([(
                "code".to_owned(),
                Json::Object(Object::from([
                    ("type".to_owned(), Json::Text("string".to_owned())),
                    (
                        "description".to_owned(),
                        Json::Text("Python code to execute".to_owned()),
                    ),
                ])),
            )])),
        ),
        (
            "required".to_owned(),
            Json::List(vec![Json::Text("code".to_owned())]),
        ),
    ]))
}
