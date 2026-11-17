use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    Object,
    String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: ToolKind,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolParameters {
    #[serde(rename = "type")]
    pub kind: SchemaKind,
    pub properties: IndexMap<String, ToolProperty>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolProperty {
    #[serde(rename = "type")]
    pub kind: SchemaKind,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageTool {
    Usage,
    Team,
    Tag,
}

impl UsageTool {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Usage => "get_usage_data",
            Self::Team => "get_team_usage_data",
            Self::Tag => "get_tag_usage_data",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Usage => "global usage data",
            Self::Team => "team usage data",
            Self::Tag => "tag usage data",
        }
    }

    #[must_use]
    pub const fn allowed_for(self, is_admin: bool) -> bool {
        match self {
            Self::Usage => true,
            Self::Team | Self::Tag => is_admin,
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "get_usage_data" => Some(Self::Usage),
            "get_team_usage_data" => Some(Self::Team),
            "get_tag_usage_data" => Some(Self::Tag),
            _ => None,
        }
    }

    #[must_use]
    pub fn definition(self) -> ToolDefinition {
        ToolDefinition {
            kind: ToolKind::Function,
            function: ToolFunction {
                name: self.name().to_owned(),
                description: self.description().to_owned(),
                parameters: self.parameters(),
            },
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Usage => {
                concat!(
                    "Fetch aggregated global usage/spend data. Returns daily spend, ",
                    "token counts, request counts, and breakdowns by model, provider, ",
                    "and API key. Use for overall spend, top models, top providers."
                )
            }
            Self::Team => {
                concat!(
                    "Fetch usage/spend data broken down by team. Use for questions ",
                    "like 'which team spends the most' or 'show me team X usage'."
                )
            }
            Self::Tag => {
                concat!(
                    "Fetch usage/spend data broken down by tag. Tags are labels ",
                    "attached to requests (features, environments, credentials)."
                )
            }
        }
    }

    const fn extra_property(self) -> (&'static str, &'static str) {
        match self {
            Self::Usage => ("user_id", "Optional user ID filter. Omit for global view."),
            Self::Team => (
                "team_ids",
                "Optional comma-separated team IDs. Omit for all teams.",
            ),
            Self::Tag => (
                "tags",
                "Optional comma-separated tag names. Omit for all tags.",
            ),
        }
    }

    fn parameters(self) -> ToolParameters {
        let fields: [(&str, &str); 3] = [
            ("start_date", "Start date in YYYY-MM-DD format"),
            ("end_date", "End date in YYYY-MM-DD format"),
            self.extra_property(),
        ];
        let properties = fields
            .into_iter()
            .map(|(name, description)| {
                (
                    name.to_owned(),
                    ToolProperty {
                        kind: SchemaKind::String,
                        description: description.to_owned(),
                    },
                )
            })
            .collect();
        ToolParameters {
            kind: SchemaKind::Object,
            properties,
            required: vec!["start_date".to_owned(), "end_date".to_owned()],
        }
    }
}

#[must_use]
pub fn tools_for_role(is_admin: bool) -> Vec<ToolDefinition> {
    let selected: &[UsageTool] = if is_admin {
        &[UsageTool::Usage, UsageTool::Team, UsageTool::Tag]
    } else {
        &[UsageTool::Usage]
    };
    selected.iter().map(|tool| tool.definition()).collect()
}
