use crate::key_repository::table::columns::TokenSqlType;

use super::TableColumn;
use crate::table_ops::access::{
    NOTIFICATION_ATTRIBUTE, NOTIFICATION_TABLE, SPEND_ATTRIBUTE, SPEND_TABLE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpendColumn {
    RequestId,
    CallType,
    ApiKey,
    Spend,
    TotalTokens,
    PromptTokens,
    CompletionTokens,
    StartTime,
    EndTime,
    RequestDurationMs,
    CompletionStartTime,
    Model,
    ModelId,
    ModelGroup,
    CustomLlmProvider,
    ApiBase,
    User,
    Metadata,
    CacheHit,
    CacheKey,
    RequestTags,
    TeamId,
    OrganizationId,
    EndUser,
    RequesterIpAddress,
    Messages,
    Response,
    SessionId,
    Status,
    McpNamespacedToolName,
    BillingAgentId,
    AgentId,
    ProxyServerRequest,
    LitellmCallId,
    CreatedAt,
    UpdatedAt,
}

impl TableColumn for SpendColumn {
    const TABLE: &'static str = SPEND_TABLE;
    const ATTRIBUTE: &'static str = SPEND_ATTRIBUTE;
    const KEY: Self = Self::RequestId;

    fn sql_definition(self) -> (&'static str, TokenSqlType) {
        match self {
            Self::RequestId => ("request_id", TokenSqlType::Text),
            Self::CallType => ("call_type", TokenSqlType::Text),
            Self::ApiKey => ("api_key", TokenSqlType::Text),
            Self::Spend => ("spend", TokenSqlType::DoublePrecision),
            Self::TotalTokens => ("total_tokens", TokenSqlType::BigInt),
            Self::PromptTokens => ("prompt_tokens", TokenSqlType::BigInt),
            Self::CompletionTokens => ("completion_tokens", TokenSqlType::BigInt),
            Self::StartTime => ("startTime", TokenSqlType::Timestamp),
            Self::EndTime => ("endTime", TokenSqlType::Timestamp),
            Self::RequestDurationMs => ("request_duration_ms", TokenSqlType::BigInt),
            Self::CompletionStartTime => ("completionStartTime", TokenSqlType::Timestamp),
            Self::Model => ("model", TokenSqlType::Text),
            Self::ModelId => ("model_id", TokenSqlType::Text),
            Self::ModelGroup => ("model_group", TokenSqlType::Text),
            Self::CustomLlmProvider => ("custom_llm_provider", TokenSqlType::Text),
            Self::ApiBase => ("api_base", TokenSqlType::Text),
            Self::User => ("user", TokenSqlType::Text),
            Self::Metadata => ("metadata", TokenSqlType::Json),
            Self::CacheHit => ("cache_hit", TokenSqlType::Text),
            Self::CacheKey => ("cache_key", TokenSqlType::Text),
            Self::RequestTags => ("request_tags", TokenSqlType::Json),
            Self::TeamId => ("team_id", TokenSqlType::Text),
            Self::OrganizationId => ("organization_id", TokenSqlType::Text),
            Self::EndUser => ("end_user", TokenSqlType::Text),
            Self::RequesterIpAddress => ("requester_ip_address", TokenSqlType::Text),
            Self::Messages => ("messages", TokenSqlType::Json),
            Self::Response => ("response", TokenSqlType::Json),
            Self::SessionId => ("session_id", TokenSqlType::Text),
            Self::Status => ("status", TokenSqlType::Text),
            Self::McpNamespacedToolName => ("mcp_namespaced_tool_name", TokenSqlType::Text),
            Self::BillingAgentId => ("billing_agent_id", TokenSqlType::Text),
            Self::AgentId => ("agent_id", TokenSqlType::Text),
            Self::ProxyServerRequest => ("proxy_server_request", TokenSqlType::Json),
            Self::LitellmCallId => ("litellm_call_id", TokenSqlType::Text),
            Self::CreatedAt => ("created_at", TokenSqlType::Timestamp),
            Self::UpdatedAt => ("updated_at", TokenSqlType::Timestamp),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotificationColumn {
    RequestId,
    UserId,
    Models,
    Justification,
    Status,
}

impl TableColumn for NotificationColumn {
    const TABLE: &'static str = NOTIFICATION_TABLE;
    const ATTRIBUTE: &'static str = NOTIFICATION_ATTRIBUTE;
    const KEY: Self = Self::RequestId;

    fn sql_definition(self) -> (&'static str, TokenSqlType) {
        match self {
            Self::RequestId => ("request_id", TokenSqlType::Text),
            Self::UserId => ("user_id", TokenSqlType::Text),
            Self::Models => ("models", TokenSqlType::TextArray),
            Self::Justification => ("justification", TokenSqlType::Text),
            Self::Status => ("status", TokenSqlType::Text),
        }
    }
}
