from datetime import datetime
from decimal import Decimal
from typing import NotRequired, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    anthropic_passthrough_logging_handler as handler_module,
)
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Handler = handler_module.AnthropicPassthroughLoggingHandler
FIXED_ID = "b3633-id"


class Input(TypedDict):
    status_code: int
    body: str
    url_route: str
    start_time: str
    request_body: dict[str, JsonValue] | None
    request_metadata: dict[str, JsonValue]


class DumpMessage(TypedDict):
    role: str | None
    content: str | None
    tool_calls: NotRequired[list[JsonValue] | None]
    function_call: NotRequired[dict[str, JsonValue] | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]


class DumpChoice(TypedDict):
    index: int
    finish_reason: str | None
    message: DumpMessage


class Dump(TypedDict):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[DumpChoice]


class KwargsView(TypedDict):
    response_cost: float
    model: str
    batch_job_state: str
    batch_id: NotRequired[str]
    unified_object_id: NotRequired[str]


class DetailsView(BaseModel):
    model: str | None = None
    response_cost: float | None = None
    batch_id: str | None = None


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str
    model_call_details: DetailsView


class OutcomeView(BaseModel):
    result: ModelResponse | None
    kwargs: dict[str, JsonValue]


RESULT: TypeAdapter[Dump] = TypeAdapter(Dump)
KWARGS: TypeAdapter[KwargsView] = TypeAdapter(KwargsView)
OUTCOME: TypeAdapter[OutcomeView] = TypeAdapter(OutcomeView)
LOGGING: TypeAdapter[LoggingView] = TypeAdapter(LoggingView)


def cost_or_none(cost: float | None) -> JsonValue:
    if cost is None:
        return None
    return format(Decimal(repr(cost)).normalize(), "f")


def make_logging(start: datetime) -> Logging:
    return Logging(
        model="",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=start,
        litellm_call_id="b3633-call",
        function_id="b3633",
    )


def message_value(message: DumpMessage) -> dict[str, JsonValue]:
    tool_calls: JsonValue = None
    if "tool_calls" in message:
        tool_calls = message["tool_calls"]
    function_call: JsonValue = None
    if "function_call" in message:
        function_call = message["function_call"]
    fields: dict[str, JsonValue] = {}
    if "provider_specific_fields" in message:
        present = message["provider_specific_fields"]
        if present is not None:
            fields = present
    return {
        "role": message["role"],
        "content": message["content"],
        "tool_calls": tool_calls,
        "function_call": function_call,
        "provider_specific_fields": fields,
    }


def choice_value(choice: DumpChoice) -> dict[str, JsonValue]:
    return {
        "finish_reason": choice["finish_reason"],
        "index": choice["index"],
        "message": message_value(choice["message"]),
    }


def result_value(response: ModelResponse | None) -> dict[str, JsonValue]:
    if response is None:
        raise ValueError("the batch handler returned no result")
    dump = RESULT.validate_json(response.model_dump_json())
    return {
        "id": FIXED_ID,
        "model": dump["model"],
        "object": dump["object"],
        "created": dump["created"],
        "choices": [choice_value(choice) for choice in dump["choices"]],
    }


def kwargs_value(
    kwargs: dict[str, JsonValue], request_metadata: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    view = KWARGS.validate_python(kwargs)
    batch_id: JsonValue = None
    if "batch_id" in view:
        batch_id = view["batch_id"]
    unified_object_id: JsonValue = None
    if "unified_object_id" in view:
        unified_object_id = view["unified_object_id"]
    return {
        "response_cost": format(Decimal(repr(view["response_cost"])).normalize(), "f"),
        "model": view["model"],
        "batch_id": batch_id,
        "unified_object_id": unified_object_id,
        "batch_job_state": view["batch_job_state"],
        "request_metadata": request_metadata,
    }


def call_log_value(logging_obj: Logging) -> dict[str, JsonValue]:
    view = LOGGING.validate_python(logging_obj)
    details = view.model_call_details
    return {
        "model": view.model,
        "details": {
            "model": details.model,
            "response_cost": cost_or_none(details.response_cost),
            "batch_id": details.batch_id,
        },
    }


def run(input_value: Input) -> dict[str, JsonValue]:
    start = datetime.fromisoformat(input_value["start_time"])
    logging_obj = make_logging(start)
    response = httpx.Response(
        input_value["status_code"], content=input_value["body"].encode()
    )
    outcome = OUTCOME.validate_python(Handler.batch_creation_handler(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L896-L907
        httpx_response=response,
        logging_obj=logging_obj,
        url_route=input_value["url_route"],
        result=input_value["body"],
        start_time=start,
        end_time=start,
        cache_hit=False,
        request_body=input_value["request_body"],
    ))
    return {
        "result": result_value(outcome.result),
        "kwargs": kwargs_value(outcome.kwargs, input_value["request_metadata"]),
        "call_log": call_log_value(logging_obj),
    }
