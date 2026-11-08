from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm import exceptions
from litellm._logging import _redact_string
from litellm.constants import MAX_BASE64_LENGTH_FOR_LOGGING
from litellm.litellm_core_utils import litellm_logging, logging_utils
from litellm.litellm_core_utils.core_helpers import is_expected_client_error
from litellm.litellm_core_utils.specialty_caches.service_trace_id_cache import (
    in_memory_trace_id_cache,
)

if TYPE_CHECKING:
    from litellm.types.utils import (
        StandardLoggingMCPToolCall,
        StandardLoggingVectorStoreRequest,
    )

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false, reportUnknownMemberType=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SECRET_PROBE = "sk-abcdefghijklmno"  # noqa: S105  # a fake key that only proves redaction is on

SETUP = litellm_logging.StandardLoggingPayloadSetup

EXCEPTION_CLASSES: dict[str, type[Exception]] = {
    "APIConnectionError": exceptions.APIConnectionError,
    "AuthenticationError": exceptions.AuthenticationError,
    "BadGatewayError": exceptions.BadGatewayError,
    "BadRequestError": exceptions.BadRequestError,
    "BlockedPiiEntityError": exceptions.BlockedPiiEntityError,
    "BudgetExceededError": exceptions.BudgetExceededError,
    "ContentPolicyViolationError": exceptions.ContentPolicyViolationError,
    "ContextWindowExceededError": exceptions.ContextWindowExceededError,
    "GuardrailRaisedException": exceptions.GuardrailRaisedException,
    "InternalServerError": exceptions.InternalServerError,
    "MidStreamFallbackError": exceptions.MidStreamFallbackError,
    "NotFoundError": exceptions.NotFoundError,
    "PermissionDeniedError": exceptions.PermissionDeniedError,
    "RateLimitError": exceptions.RateLimitError,
    "ServiceUnavailableError": exceptions.ServiceUnavailableError,
    "Timeout": exceptions.Timeout,
    "UnprocessableEntityError": exceptions.UnprocessableEntityError,
    "UnsupportedParamsError": exceptions.UnsupportedParamsError,
    "RuntimeError": RuntimeError,
    "TypeError": TypeError,
    "ValueError": ValueError,
}

ATTRIBUTE_KEYS: tuple[tuple[str, str], ...] = (
    ("code", "code"),
    ("status_code", "status_code"),
    ("llm_provider", "llm_provider"),
    ("category", "category"),
    ("rate_limit_type", "rate_limit_type"),
    ("budget_entity_type", "entity_type"),
    ("budget_entity_id", "entity_id"),
    ("budget_limit", "max_budget"),
    ("budget_spend", "current_cost"),
    ("proxy_type", "type"),
    ("headers", "headers"),
    ("litellm_response_headers", "litellm_response_headers"),
)


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "merge_litellm_metadata":
        return _merge(input_value)
    if function == "standard_metadata":
        return _standard_metadata(input_value)
    if function == "error_information":
        return _error_information(input_value)
    if function == "request_tags":
        return _request_tags(input_value)
    if function == "trace_session":
        return _trace_session(input_value)
    if function == "truncate_base64":
        return _truncate_base64(input_value)
    raise ValueError(f"unknown logging_metadata function {function}")


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _optional_object(value: Json, name: str) -> dict[str, Json] | None:
    if value is None:
        return None
    return _object(value, name)


def _optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string_list(value: Json, name: str) -> list[str] | None:
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [item if isinstance(item, str) else _not_a_string(name) for item in value]


def _not_a_string(name: str) -> str:
    raise ValueError(f"{name} must hold strings")


def _merge(input_value: dict[str, Json]) -> Json:
    params = _object(input_value.get("litellm_params"), "litellm_params")
    merged = cast(Json, SETUP.merge_litellm_metadata(params))
    scrubbed = cast(Json, litellm_logging.scrub_sensitive_keys_in_metadata(dict(params)))
    return {"merged": merged, "scrubbed": scrubbed}


def _standard_metadata(input_value: dict[str, Json]) -> Json:
    metadata = _optional_object(input_value.get("metadata"), "metadata")
    params = _optional_object(input_value.get("litellm_params"), "litellm_params")
    applied_guardrails = _optional_string_list(
        input_value.get("applied_guardrails"), "applied_guardrails"
    )
    mcp_tool_call = cast(
        "StandardLoggingMCPToolCall | None",
        _optional_object(input_value.get("mcp_tool_call_metadata"), "mcp_tool_call_metadata"),
    )
    vector_store = cast(
        "list[StandardLoggingVectorStoreRequest] | None",
        input_value.get("vector_store_request_metadata"),
    )
    usage_object = _optional_object(input_value.get("usage_object"), "usage_object")
    prompt_integration = _optional_string(
        input_value.get("prompt_integration"), "prompt_integration"
    )
    custom_llm_provider = _optional_string(
        input_value.get("custom_llm_provider"), "custom_llm_provider"
    )
    record = cast(
        Json,
        SETUP.get_standard_logging_metadata(
            metadata=metadata,
            litellm_params=params,
            prompt_integration=prompt_integration,
            applied_guardrails=applied_guardrails,
            mcp_tool_call_metadata=mcp_tool_call,
            vector_store_request_metadata=vector_store,
            usage_object=usage_object,
            custom_llm_provider=custom_llm_provider,
        ),
    )
    public_record = cast(
        Json,
        litellm_logging.get_standard_logging_metadata(metadata),
    )
    return {"record": record, "public_record": public_record}


def _exception(input_value: dict[str, Json]) -> Exception:
    class_name = _optional_string(input_value.get("class_name"), "class_name") or ""
    semantic = _optional_string(input_value.get("class"), "class")
    base_name = semantic if semantic is not None else class_name
    base = EXCEPTION_CLASSES.get(base_name, Exception)
    exception_type = base if base.__name__ == class_name else type(class_name, (base,), {})
    error = Exception.__new__(exception_type)
    attributes = error.__dict__
    message = _optional_string(input_value.get("message"), "message")
    if message is not None:
        attributes["message"] = message
    for input_name, attribute_name in ATTRIBUTE_KEYS:
        value = input_value.get(input_name)
        if value is not None:
            attributes[attribute_name] = value
    response = _response(input_value)
    if response is not None:
        attributes["response"] = response
    return error


def _response(input_value: dict[str, Json]) -> SimpleNamespace | None:
    status_code = input_value.get("response_status_code")
    headers = _optional_object(input_value.get("response_headers"), "response_headers")
    if status_code is None and headers is None:
        return None
    response = SimpleNamespace()
    if status_code is not None:
        response.__dict__["status_code"] = status_code
    if headers is not None:
        response.__dict__["headers"] = headers
    return response


def _error_information(input_value: dict[str, Json]) -> Json:
    exception_value = input_value.get("exception")
    error = None if exception_value is None else _exception(_object(exception_value, "exception"))
    traceback_str = _optional_string(input_value.get("traceback"), "traceback")
    error_str = _optional_string(input_value.get("error_str"), "error_str")
    metadata = _optional_object(input_value.get("metadata"), "metadata") or {}
    policy = _optional_object(input_value.get("policy"), "policy") or {}
    litellm.log_client_error_tracebacks = policy.get("log_client_error_tracebacks") is True
    expected = policy.get("expected_client_error") is True
    if is_expected_client_error(error) != expected:
        raise ValueError("policy.expected_client_error must match LiteLLM's own predicate")
    redaction_enabled = _redact_string(SECRET_PROBE) == "REDACTED"
    if redaction_enabled != (input_value.get("redact") is True):
        raise ValueError("the case env and the redact input must agree")
    information = cast(
        Json,
        SETUP.get_error_information(
            original_exception=error,
            traceback_str=traceback_str,
        ),
    )
    payload_result = SETUP.get_error_information_for_logging_payload(
        metadata=metadata,
        original_exception=error,
        error_str=error_str,
    )
    payload_information, payload_error_str = payload_result
    return {
        "information": information,
        "payload_information": cast(Json, payload_information),
        "error_str": payload_error_str,
    }


def _request_tags(input_value: dict[str, Json]) -> Json:
    params = _object(input_value.get("litellm_params"), "litellm_params")
    headers = _optional_object(input_value.get("headers"), "headers")
    extra = (
        _optional_string_list(
            input_value.get("extra_spend_tag_headers"), "extra_spend_tag_headers"
        )
        or []
    )
    disable = input_value.get("disable_add_user_agent") is True
    litellm.disable_add_user_agent_to_request_tags = disable
    litellm.extra_spend_tag_headers = extra
    proxy_server_request: dict[str, object] = {}
    if headers is not None:
        proxy_server_request["headers"] = headers
    tags = SETUP._get_request_tags(params, proxy_server_request)
    return {"tags": cast(Json, tags)}


def _trace_session(input_value: dict[str, Json]) -> Json:
    params = _object(input_value.get("litellm_params"), "litellm_params")
    correlation = input_value.get("correlation") is True
    fallback_trace_id = (
        _optional_string(input_value.get("fallback_trace_id"), "fallback_trace_id") or ""
    )
    fallback_session_id = (
        _optional_string(input_value.get("fallback_session_id"), "fallback_session_id") or ""
    )
    call_id = _optional_string(input_value.get("call_id"), "call_id") or ""
    cached_trace_id = _optional_string(input_value.get("cached_trace_id"), "cached_trace_id")
    litellm.request_correlation_in_logs = correlation
    logging_obj = cast(
        litellm_logging.Logging,
        SimpleNamespace(
            litellm_trace_id=fallback_trace_id,
            litellm_session_id=fallback_session_id,
            litellm_call_id=call_id,
        ),
    )
    trace_id = SETUP.get_standard_logging_payload_trace_id(logging_obj, params)
    session_id = SETUP.get_standard_logging_payload_session_id(logging_obj, params)
    if cached_trace_id is not None:
        in_memory_trace_id_cache.set_cache(
            litellm_call_id=call_id,
            service_name="langfuse",
            trace_id=cached_trace_id,
        )
    cached = litellm_logging.Logging._get_trace_id(logging_obj, "langfuse")
    return {"trace_id": trace_id, "session_id": session_id, "cached_trace_id": cached}


def _truncate_base64(input_value: dict[str, Json]) -> Json:
    max_length = input_value.get("max_base64_length")
    if not isinstance(max_length, int):
        raise ValueError("max_base64_length must be an integer")
    if max_length != MAX_BASE64_LENGTH_FOR_LOGGING:
        raise ValueError("the case env must set MAX_BASE64_LENGTH_FOR_LOGGING")
    messages = cast("str | list[object] | dict[str, object] | None", input_value.get("messages"))
    truncated = cast(
        Json,
        logging_utils.truncate_base64_in_messages(messages),
    )
    return {"messages": truncated}
