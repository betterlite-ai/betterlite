import builtins
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
    from collections.abc import Callable

    from litellm.types.utils import (
        StandardLoggingMCPToolCall,
        StandardLoggingVectorStoreRequest,
    )

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false, reportUnknownMemberType=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SECRET_PROBE = "sk-abcdefghijklmno"  # noqa: S105  # a fake key that only proves redaction is on

SETUP = litellm_logging.StandardLoggingPayloadSetup

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
    handlers: dict[Json, Callable[[dict[str, Json]], Json]] = {
        "merge_litellm_metadata": _merge,
        "standard_metadata": _standard_metadata,
        "error_information": _error_information,
        "request_tags": _request_tags,
        "trace_session": _trace_session,
        "truncate_base64": _truncate_base64,
    }
    function = input_value.get("function")
    if function not in handlers:
        raise ValueError(f"unknown logging_metadata function {function}")
    return handlers[function](input_value)


def _merge(input_value: dict[str, Json]) -> Json:
    params = cast("dict[str, Json]", input_value["litellm_params"])
    merged = cast(Json, SETUP.merge_litellm_metadata(params))
    scrubbed = cast(Json, litellm_logging.scrub_sensitive_keys_in_metadata(dict(params)))
    return {"merged": merged, "scrubbed": scrubbed}


def _standard_metadata(input_value: dict[str, Json]) -> Json:
    metadata = cast("dict[str, Json] | None", input_value.get("metadata"))
    record = SETUP.get_standard_logging_metadata(
        metadata=metadata,
        litellm_params=cast("dict[str, Json] | None", input_value.get("litellm_params")),
        prompt_integration=cast("str | None", input_value.get("prompt_integration")),
        applied_guardrails=cast("list[str] | None", input_value.get("applied_guardrails")),
        mcp_tool_call_metadata=cast(
            "StandardLoggingMCPToolCall | None", input_value.get("mcp_tool_call_metadata")
        ),
        vector_store_request_metadata=cast(
            "list[StandardLoggingVectorStoreRequest] | None",
            input_value.get("vector_store_request_metadata"),
        ),
        usage_object=cast("dict[str, Json] | None", input_value.get("usage_object")),
        custom_llm_provider=cast("str | None", input_value.get("custom_llm_provider")),
    )
    public_record = litellm_logging.get_standard_logging_metadata(metadata)
    return {"record": cast(Json, record), "public_record": cast(Json, public_record)}


def _exception(input_value: dict[str, Json]) -> Exception:
    class_name = cast("str", input_value.get("class_name") or "")
    base_name = cast("str", input_value.get("class") or class_name)
    base = cast(
        "type[Exception]",
        getattr(exceptions, base_name, None) or getattr(builtins, base_name, Exception),
    )
    exception_type = base if base.__name__ == class_name else type(class_name, (base,), {})
    error = Exception.__new__(exception_type)
    attributes = error.__dict__
    if input_value.get("message") is not None:
        attributes["message"] = input_value["message"]
    for input_name, attribute_name in ATTRIBUTE_KEYS:
        if input_value.get(input_name) is not None:
            attributes[attribute_name] = input_value[input_name]
    status_code = input_value.get("response_status_code")
    headers = input_value.get("response_headers")
    if status_code is not None or headers is not None:
        attributes["response"] = SimpleNamespace(
            **{
                name: value
                for name, value in (("status_code", status_code), ("headers", headers))
                if value is not None
            }
        )
    return error


def _error_information(input_value: dict[str, Json]) -> Json:
    exception_value = cast("dict[str, Json] | None", input_value.get("exception"))
    error = None if exception_value is None else _exception(exception_value)
    policy = cast("dict[str, Json]", input_value.get("policy") or {})
    litellm.log_client_error_tracebacks = policy.get("log_client_error_tracebacks") is True
    if is_expected_client_error(error) != (policy.get("expected_client_error") is True):
        raise ValueError("policy.expected_client_error must match LiteLLM's own predicate")
    if (_redact_string(SECRET_PROBE) == "REDACTED") != (input_value.get("redact") is True):
        raise ValueError("the case env and the redact input must agree")
    information = SETUP.get_error_information(
        original_exception=error,
        traceback_str=cast("str | None", input_value.get("traceback")),
    )
    payload_information, payload_error_str = SETUP.get_error_information_for_logging_payload(
        metadata=cast("dict[str, Json]", input_value.get("metadata") or {}),
        original_exception=error,
        error_str=cast("str | None", input_value.get("error_str")),
    )
    return {
        "information": cast(Json, information),
        "payload_information": cast(Json, payload_information),
        "error_str": payload_error_str,
    }


def _request_tags(input_value: dict[str, Json]) -> Json:
    headers = input_value.get("headers")
    litellm.disable_add_user_agent_to_request_tags = (
        input_value.get("disable_add_user_agent") is True
    )
    litellm.extra_spend_tag_headers = cast(
        "list[str]", input_value.get("extra_spend_tag_headers") or []
    )
    proxy_server_request: dict[str, object] = {} if headers is None else {"headers": headers}
    tags = SETUP._get_request_tags(
        cast("dict[str, Json]", input_value["litellm_params"]), proxy_server_request
    )
    return {"tags": cast(Json, tags)}


def _trace_session(input_value: dict[str, Json]) -> Json:
    params = cast("dict[str, Json]", input_value["litellm_params"])
    call_id = cast("str", input_value.get("call_id") or "")
    cached_trace_id = cast("str | None", input_value.get("cached_trace_id"))
    litellm.request_correlation_in_logs = input_value.get("correlation") is True
    logging_obj = cast(
        litellm_logging.Logging,
        SimpleNamespace(
            litellm_trace_id=input_value.get("fallback_trace_id") or "",
            litellm_session_id=input_value.get("fallback_session_id") or "",
            litellm_call_id=call_id,
        ),
    )
    trace_id = SETUP.get_standard_logging_payload_trace_id(logging_obj, params)
    session_id = SETUP.get_standard_logging_payload_session_id(logging_obj, params)
    if cached_trace_id is not None:
        in_memory_trace_id_cache.set_cache(
            litellm_call_id=call_id, service_name="langfuse", trace_id=cached_trace_id
        )
    cached = litellm_logging.Logging._get_trace_id(logging_obj, "langfuse")
    return {"trace_id": trace_id, "session_id": session_id, "cached_trace_id": cached}


def _truncate_base64(input_value: dict[str, Json]) -> Json:
    if input_value.get("max_base64_length") != MAX_BASE64_LENGTH_FOR_LOGGING:
        raise ValueError("the case env must set MAX_BASE64_LENGTH_FOR_LOGGING")
    messages = cast("str | list[object] | dict[str, object] | None", input_value.get("messages"))
    return {"messages": cast(Json, logging_utils.truncate_base64_in_messages(messages))}
