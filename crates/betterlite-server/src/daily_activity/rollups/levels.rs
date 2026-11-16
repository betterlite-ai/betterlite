use betterlite_analytics::daily_metrics::{BreakdownMetrics, MetricWithMetadata};
use betterlite_db::daily_activity_contracts::GroupingSetsRow;
use indexmap::IndexMap;

pub(super) const GROUP_GRAND_TOTAL: i64 = 127;
pub(super) const GROUP_DATE: i64 = 63;
pub(super) const GROUP_DATE_API_KEY: i64 = 31;
pub(super) const GROUP_DATE_MODEL: i64 = 47;
pub(super) const GROUP_DATE_MODEL_API_KEY: i64 = 15;
pub(super) const GROUP_DATE_MODEL_GROUP: i64 = 55;
pub(super) const GROUP_DATE_MODEL_GROUP_API_KEY: i64 = 23;
pub(super) const GROUP_DATE_PROVIDER: i64 = 59;
pub(super) const GROUP_DATE_PROVIDER_API_KEY: i64 = 27;
pub(super) const GROUP_DATE_MCP: i64 = 61;
pub(super) const GROUP_DATE_MCP_API_KEY: i64 = 29;
pub(super) const GROUP_DATE_ENDPOINT: i64 = 62;
pub(super) const GROUP_DATE_ENDPOINT_API_KEY: i64 = 30;

#[derive(Clone, Copy)]
pub(super) enum Dimension {
    Model,
    ModelGroup,
    Provider,
    Mcp,
    Endpoint,
}

#[derive(Clone, Copy)]
pub(super) enum RollupLevel {
    GrandTotal,
    Daily,
    ApiKeys,
    Dimension(Dimension),
    Keyed(Dimension),
    Other,
}

pub(super) const fn rollup_level(group_level: i64) -> RollupLevel {
    match group_level {
        GROUP_GRAND_TOTAL => RollupLevel::GrandTotal,
        GROUP_DATE => RollupLevel::Daily,
        GROUP_DATE_API_KEY => RollupLevel::ApiKeys,
        GROUP_DATE_MODEL => RollupLevel::Dimension(Dimension::Model),
        GROUP_DATE_MODEL_GROUP => RollupLevel::Dimension(Dimension::ModelGroup),
        GROUP_DATE_PROVIDER => RollupLevel::Dimension(Dimension::Provider),
        GROUP_DATE_MCP => RollupLevel::Dimension(Dimension::Mcp),
        GROUP_DATE_ENDPOINT => RollupLevel::Dimension(Dimension::Endpoint),
        GROUP_DATE_MODEL_API_KEY => RollupLevel::Keyed(Dimension::Model),
        GROUP_DATE_MODEL_GROUP_API_KEY => RollupLevel::Keyed(Dimension::ModelGroup),
        GROUP_DATE_PROVIDER_API_KEY => RollupLevel::Keyed(Dimension::Provider),
        GROUP_DATE_MCP_API_KEY => RollupLevel::Keyed(Dimension::Mcp),
        GROUP_DATE_ENDPOINT_API_KEY => RollupLevel::Keyed(Dimension::Endpoint),
        _ => RollupLevel::Other,
    }
}

impl Dimension {
    pub(super) fn name(self, record: &GroupingSetsRow) -> Option<&str> {
        match self {
            Self::Model => record.model.as_deref(),
            Self::ModelGroup => record.model_group.as_deref(),
            Self::Provider => Some(record.custom_llm_provider.as_deref().unwrap_or("unknown")),
            Self::Mcp => record.mcp_namespaced_tool_name.as_deref(),
            Self::Endpoint => record.endpoint.as_deref(),
        }
    }

    pub(super) const fn target(
        self,
        breakdown: &mut BreakdownMetrics,
    ) -> &mut IndexMap<String, MetricWithMetadata> {
        match self {
            Self::Model => &mut breakdown.models,
            Self::ModelGroup => &mut breakdown.model_groups,
            Self::Provider => &mut breakdown.providers,
            Self::Mcp => &mut breakdown.mcp_servers,
            Self::Endpoint => &mut breakdown.endpoints,
        }
    }
}
