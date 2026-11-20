from typing import Final

from litellm.types.utils import StandardLoggingPayload
from pydantic import JsonValue, TypeAdapter

from .calls import PayloadFields

PAYLOAD: Final[TypeAdapter[StandardLoggingPayload]] = TypeAdapter(StandardLoggingPayload)

METADATA_FIELDS: Final[tuple[str, ...]] = (
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_spend",
    "user_api_key_max_budget",
    "user_api_key_budget_reset_at",
    "user_api_key_user_spend",
    "user_api_key_user_max_budget",
    "user_api_key_team_spend",
    "user_api_key_team_max_budget",
    "user_api_key_org_id",
    "user_api_key_org_alias",
    "user_api_key_team_id",
    "user_api_key_project_id",
    "user_api_key_project_alias",
    "user_api_key_user_id",
    "user_api_key_user_email",
    "user_api_key_team_alias",
    "user_api_key_end_user_id",
    "user_api_key_request_route",
    "user_api_key_auth_metadata",
    "spend_logs_metadata",
    "requester_ip_address",
    "user_agent",
    "requester_metadata",
    "requester_custom_headers",
    "prompt_management_metadata",
    "mcp_tool_call_metadata",
    "vector_store_request_metadata",
    "routing_decision",
    "applied_guardrails",
    "usage_object",
    "cold_storage_object_key",
    "team_alias",
    "team_id",
    "used_client_oauth_token",
)

HIDDEN_FIELDS: Final[tuple[str, ...]] = (
    "model_id",
    "cache_key",
    "api_base",
    "response_cost",
    "litellm_overhead_time_ms",
    "additional_headers",
    "batch_models",
    "batch_successful_requests",
    "batch_failed_requests",
    "litellm_model_name",
    "usage_object",
)


def build_payload(call: PayloadFields) -> StandardLoggingPayload:
    metadata: dict[str, JsonValue] = {name: None for name in METADATA_FIELDS}
    metadata["usage_object"] = call["usage_object"]
    doc: dict[str, JsonValue] = {
        "id": "payload",
        "trace_id": "trace",
        "session_id": "session",
        "litellm_call_id": None,
        "call_type": "completion",
        "stream": None,
        "response_cost": call["response_cost"],
        "cost_breakdown": None,
        "autorouter_savings": None,
        "autorouter_savings_estimate": None,
        "autorouter_baseline_observation": None,
        "response_cost_failure_debug_info": None,
        "status": "success",
        "status_fields": {},
        "custom_llm_provider": None,
        "total_tokens": call["total_tokens"],
        "prompt_tokens": call["prompt_tokens"],
        "completion_tokens": call["completion_tokens"],
        "startTime": 0.0,
        "endTime": 0.0,
        "completionStartTime": 0.0,
        "response_time": 0.0,
        "model_map_information": {"model_map_key": "gpt-4o", "model_map_value": None},
        "model": "gpt-4o",
        "model_id": None,
        "model_group": None,
        "api_base": "",
        "metadata": metadata,
        "cache_hit": call["cache_hit"],
        "cache_key": None,
        "saved_cache_cost": 0.0,
        "request_tags": [],
        "end_user": None,
        "requester_ip_address": None,
        "user_agent": None,
        "messages": None,
        "response": None,
        "error_str": None,
        "error_information": None,
        "model_parameters": {},
        "hidden_params": {name: None for name in HIDDEN_FIELDS},
        "guardrail_information": None,
        "standard_built_in_tools_params": None,
    }
    return PAYLOAD.validate_python(doc)
