import json
from typing import Final, Literal, TypedDict

from litellm.types.integrations.prometheus import UserAPIKeyLabelValues
from pydantic import JsonValue, TypeAdapter

from .probe import ProbeLogger


class Input(TypedDict):
    function: Literal[
        "request_and_spend",
        "zero_cost",
        "token_metrics",
        "token_detail",
        "media",
        "cache",
        "provider_cache_tokens",
        "cache_write_tokens",
    ]
    values: dict[str, str | None]
    supported_labels: dict[str, list[str]]
    response_cost: float
    total_tokens: int
    prompt_tokens: int
    completion_tokens: int
    cache_hit: bool | None
    usage_object: dict[str, JsonValue] | None
    provider_usage: dict[str, JsonValue]
    prompt_details: dict[str, JsonValue]


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

LABELS: Final[TypeAdapter[UserAPIKeyLabelValues]] = TypeAdapter(UserAPIKeyLabelValues)


def build_payload(call: Input) -> dict[str, JsonValue]:
    doc = body_fields(call)
    doc["metadata"] = metadata_fields(call)
    doc["hidden_params"] = hidden_fields()
    return doc


def metadata_fields(call: Input) -> dict[str, JsonValue]:
    metadata: dict[str, JsonValue] = {}
    for name in METADATA_FIELDS:
        metadata[name] = None
    metadata["usage_object"] = call["usage_object"]
    return metadata


def hidden_fields() -> dict[str, JsonValue]:
    hidden: dict[str, JsonValue] = {}
    for name in HIDDEN_FIELDS:
        hidden[name] = None
    return hidden


def body_fields(call: Input) -> dict[str, JsonValue]:
    return {
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
        "guardrail_information": None,
        "standard_built_in_tools_params": None,
    }


def probe(input_value: Input) -> tuple[ProbeLogger, UserAPIKeyLabelValues]:
    logger = ProbeLogger(input_value["supported_labels"], [])
    return logger, LABELS.validate_json(json.dumps(input_value["values"]))


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "request_and_spend":
            logger, labels = probe(input_value)
            logger.request_spend(labels, input_value["response_cost"])
            answer = {"events": logger.entries()}
        case "zero_cost":
            logger, labels = probe(input_value)
            logger.zero_cost(labels)
            answer = {"events": logger.entries()}
        case "token_metrics":
            logger, labels = probe(input_value)
            logger.token_metrics(build_payload(input_value), labels)
            answer = {"events": logger.entries()}
        case "token_detail":
            logger, labels = probe(input_value)
            logger.token_detail(build_payload(input_value), labels)
            answer = {"events": logger.entries()}
        case "media":
            logger, labels = probe(input_value)
            logger.media(build_payload(input_value), labels)
            answer = {"events": logger.entries()}
        case "cache":
            logger, labels = probe(input_value)
            logger.cache(build_payload(input_value), labels)
            answer = {"events": logger.entries()}
        case "provider_cache_tokens":
            read, creation = ProbeLogger.provider_cache(input_value["provider_usage"])
            answer = {"read": read, "creation": creation}
        case "cache_write_tokens":
            answer = {"cache_write_tokens": ProbeLogger.cache_write(input_value["prompt_details"])}
    return answer
