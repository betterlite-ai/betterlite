#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SupportedDbObjectType {
    Models,
    Mcp,
    Guardrails,
    Policies,
    VectorStores,
    PassThroughEndpoints,
    Prompts,
    ModelCostMap,
    Tools,
    ConfigOverrides,
    WebsearchInterceptionSettings,
}
