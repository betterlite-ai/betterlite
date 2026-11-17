import json
from datetime import datetime
from typing import Final, NotRequired, TypedDict

import litellm
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.redact_messages import redacted_standard_logging_payload
from litellm.types.utils import (
    CostBreakdown,
    StandardBuiltInToolsParams,
    StandardLoggingPayloadStatus,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
COST_BREAKDOWN: Final = TypeAdapter(CostBreakdown)
TOOLS: Final = TypeAdapter(StandardBuiltInToolsParams)


class InitKwargs(TypedDict):
    litellm_session_id: NotRequired[str]


class Init(TypedDict):
    model: str
    call_type: str
    start_time: str
    litellm_call_id: str
    function_id: str
    messages: NotRequired[list[JsonValue]]
    stream: NotRequired[bool]
    litellm_trace_id: NotRequired[str]
    litellm_params: NotRequired[dict[str, JsonValue]]
    applied_guardrails: NotRequired[list[JsonValue]]
    kwargs: NotRequired[InitKwargs]


class Settings(TypedDict):
    turn_off_message_logging: bool
    request_correlation_in_logs: bool
    log_client_error_tracebacks: bool


class Input(TypedDict):
    init: Init
    model_call_details: dict[str, JsonValue]
    response: JsonValue
    start_time: str
    end_time: str
    status: StandardLoggingPayloadStatus
    completion_start_time: NotRequired[str]
    error_str: NotRequired[str]
    standard_built_in_tools_params: NotRequired[dict[str, JsonValue]]
    response_timing_metrics: NotRequired[dict[str, float]]
    cost_breakdown: NotRequired[dict[str, JsonValue]]
    settings: NotRequired[Settings]
    redact_standard_payload: NotRequired[bool]


def run(input_value: Input) -> JsonValue:
    settings = input_value.get("settings")
    litellm.turn_off_message_logging = (
        settings["turn_off_message_logging"] if settings is not None else False
    )
    litellm.request_correlation_in_logs = (
        settings["request_correlation_in_logs"] if settings is not None else False
    )
    litellm.log_client_error_tracebacks = (
        settings["log_client_error_tracebacks"] if settings is not None else False
    )
    details = _details(input_value)
    logging_obj = _logging(input_value, details)
    tools = input_value.get("standard_built_in_tools_params")
    payload = litellm_logging.get_standard_logging_object_payload(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L6485
        kwargs=details,
        init_response_obj=input_value["response"],
        start_time=datetime.fromisoformat(input_value["start_time"]),
        end_time=datetime.fromisoformat(input_value["end_time"]),
        logging_obj=logging_obj,
        status=input_value["status"],
        error_str=input_value.get("error_str"),
        original_exception=None,
        standard_built_in_tools_params=(
            TOOLS.validate_python(tools) if tools is not None else None
        ),
    )
    if payload is None:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L6485
        return None
    if input_value.get("redact_standard_payload") is True:
        redacted = redacted_standard_logging_payload(payload)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/redact_messages.py#L175
        return JSON.validate_json(json.dumps(redacted, default=str))
    return JSON.validate_json(json.dumps(payload, default=str))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L6485


# update_environment_variables writes these keys into model_call_details before the payload is built
def _details(input_value: Input) -> dict[str, JsonValue | datetime]:
    init = input_value["init"]
    completion = input_value.get("completion_start_time")
    params = init.get("litellm_params")
    return {
        **input_value["model_call_details"],
        "model": init["model"],
        "messages": init.get("messages"),
        "call_type": init["call_type"],
        "litellm_call_id": init["litellm_call_id"],
        "litellm_params": params if params is not None else {},
        "stream": init.get("stream"),
        "applied_guardrails": init.get("applied_guardrails"),
        "completion_start_time": (
            None if completion is None else datetime.fromisoformat(completion)
        ),
    }


def _logging(
    input_value: Input, details: dict[str, JsonValue | datetime]
) -> litellm_logging.Logging:
    init = input_value["init"]
    logging_obj = litellm_logging.Logging(
        model=init["model"],
        messages=init.get("messages"),
        stream=init.get("stream"),
        call_type=init["call_type"],
        start_time=datetime.fromisoformat(init["start_time"]),
        litellm_call_id=init["litellm_call_id"],
        function_id=init["function_id"],
        litellm_trace_id=init.get("litellm_trace_id"),
    )
    logging_obj.model_call_details = details
    logging_obj.litellm_session_id = _session(init)
    metrics = input_value.get("response_timing_metrics")
    logging_obj.response_timing_metrics = metrics if metrics is not None else {}
    breakdown = input_value.get("cost_breakdown")
    if breakdown is not None:
        logging_obj.cost_breakdown = COST_BREAKDOWN.validate_python(breakdown)
    return logging_obj


def _session(init: Init) -> str:
    kwargs = init.get("kwargs")
    if kwargs is None:
        return ""
    session = kwargs.get("litellm_session_id")
    return session if session is not None else ""
