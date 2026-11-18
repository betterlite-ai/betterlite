import os
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from litellm.llms.anthropic.pass_through.utils import (
    is_reasoning_auto_summary_enabled,
    litellm_logging_obj_from_kwargs,
    local_model_name,
    normalize_reasoning_effort_value,
    prompt_cache_key_from_user_id,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter

ENV = "LITELLM_REASONING_AUTO_SUMMARY"
RESPONSE_FIELDS: Final = TypeAdapter(dict[str, JsonValue])
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class LoggingFixture(Logging):
    def __init__(self) -> None:
        details: dict[str, JsonValue] = {}
        self.model_call_details = details
        self.details = details


@runtime_checkable
class ResponseConfig(Protocol):
    # the response type carries Any, so the config enters through this JSON-facing signature
    def transform_anthropic_messages_response(  # ast-grep-ignore: no-long-names
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> JsonValue: ...


RESPONSE_CONFIG: Final = TypeAdapter[ResponseConfig](ResponseConfig, config=ARBITRARY)


class ResponseCall(TypedDict):
    function: Literal["response"]
    model: str
    status: int
    body: str


class SessionCall(TypedDict):
    function: Literal["session_cache_key"]
    user_id: JsonValue


class LocalNameCall(TypedDict):
    function: Literal["local_model_name"]
    model: str
    custom_llm_provider: JsonValue


class LoggingCall(TypedDict):
    function: Literal["logging_object"]
    litellm_logging_obj: Literal["supplied", "null", "absent", "lookalike"]


class SummaryCall(TypedDict):
    function: Literal["auto_summary"]
    global_value: bool | None
    environment: str | None


class EffortCall(TypedDict):
    function: Literal["normalize_effort"]
    effort: str
    model: str
    custom_llm_provider: str | None


Call = (
    ResponseCall | SessionCall | LocalNameCall | LoggingCall | SummaryCall | EffortCall
)


class Input(TypedDict):
    calls: list[Call]
    model_cost: dict[str, dict[str, JsonValue]]


def response_answer(call: ResponseCall) -> JsonValue:
    reply = httpx.Response(status_code=call["status"], content=call["body"].encode())
    config = RESPONSE_CONFIG.validate_python(AnthropicMessagesConfig())
    transformed = config.transform_anthropic_messages_response(
        model=call["model"], raw_response=reply, logging_obj=LoggingFixture()
    )
    return {"response": RESPONSE_FIELDS.validate_python(transformed), "model": call["model"]}


def logging_answer(call: LoggingCall) -> JsonValue:
    kind = call["litellm_logging_obj"]
    kwargs: dict[str, Logging | dict[str, JsonValue] | None] = {}
    if kind == "supplied":
        kwargs["litellm_logging_obj"] = LoggingFixture()
    elif kind == "lookalike":
        kwargs["litellm_logging_obj"] = {"model_call_details": {}}
    elif kind == "null":
        kwargs["litellm_logging_obj"] = None
    found = litellm_logging_obj_from_kwargs(kwargs)
    candidate = kwargs.get("litellm_logging_obj")
    returned = "supplied" if found is not None and found is candidate else "none"
    return {"returned": returned}


def set_environment(value: str | None) -> None:
    if value is None:
        os.environ.pop(ENV, None)
    else:
        os.environ[ENV] = value


def summary_answer(call: SummaryCall) -> JsonValue:
    previous = os.environ.get(ENV)
    set_environment(call["environment"])
    try:
        litellm.reasoning_auto_summary = call["global_value"] is True
        return {"enabled": is_reasoning_auto_summary_enabled()}
    finally:
        set_environment(previous)


def effort_answer(
    call: EffortCall, model_cost: dict[str, dict[str, JsonValue]]
) -> JsonValue:
    litellm.model_cost = dict(model_cost)
    effort = normalize_reasoning_effort_value(
        call["effort"], call["model"], call["custom_llm_provider"]
    )
    return {"effort": effort}


def answer(call: Call, model_cost: dict[str, dict[str, JsonValue]]) -> JsonValue:
    match call["function"]:
        case "response":
            return response_answer(call)
        case "session_cache_key":
            return {"key": prompt_cache_key_from_user_id(call["user_id"])}
        case "local_model_name":
            return {"value": local_model_name(call["model"], call["custom_llm_provider"])}
        case "logging_object":
            return logging_answer(call)
        case "auto_summary":
            return summary_answer(call)
        case "normalize_effort":
            return effort_answer(call, model_cost)


def run(input_value: Input) -> JsonValue:
    answers = [answer(call, input_value["model_cost"]) for call in input_value["calls"]]
    return {"calls": answers}
