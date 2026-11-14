from enum import Enum

from betterlite.types.integrations.prometheus.core import (
    EXCEPTION_CLASS,
    EXCEPTION_STATUS,
    RATE_LIMIT_CATEGORY,
    RATE_LIMIT_TYPE,
    REQUESTED_MODEL,
)


class UserAPIKeyLabelNames(Enum):
    END_USER = "end_user"
    USER = "user"
    USER_EMAIL = "user_email"
    USER_ALIAS = "user_alias"
    API_KEY_HASH = "hashed_api_key"
    API_KEY_ALIAS = "api_key_alias"
    TEAM = "team"
    TEAM_ALIAS = "team_alias"
    REQUESTED_MODEL = REQUESTED_MODEL
    v1_LITELLM_MODEL_NAME = "model"
    v2_LITELLM_MODEL_NAME = "litellm_model_name"
    TAG = "tag"
    MODEL_ID = "model_id"
    API_BASE = "api_base"
    API_PROVIDER = "api_provider"
    EXCEPTION_STATUS = EXCEPTION_STATUS
    EXCEPTION_CLASS = EXCEPTION_CLASS
    RATE_LIMIT_CATEGORY = RATE_LIMIT_CATEGORY
    RATE_LIMIT_TYPE = RATE_LIMIT_TYPE
    STATUS_CODE = "status_code"
    FALLBACK_MODEL = "fallback_model"
    ROUTE = "route"
    MODEL_GROUP = "model_group"
    CLIENT_IP = "client_ip"
    USER_AGENT = "user_agent"
    CALLBACK_NAME = "callback_name"
    STREAM = "stream"
    ORG_ID = "org_id"
    ORG_ALIAS = "org_alias"
    MCP_TOOL_NAME = "mcp_tool_name"
    MCP_SERVER_NAME = "mcp_server_name"
    SERVICE_TIER = "service_tier"
    INPUT_SEQUENCE_LENGTH = "input_sequence_length"


__all__ = ["UserAPIKeyLabelNames"]
