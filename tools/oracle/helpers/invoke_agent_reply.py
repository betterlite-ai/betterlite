import datetime as dt
import os
from typing import TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.chat.invoke_agent.transformation import AmazonInvokeAgentConfig
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.llms.bedrock_invoke_agents import (
    InvokeAgentEvent,
    InvokeAgentEventHeaders,
    InvokeAgentEventPayload,
)
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, JsonValue, TypeAdapter

os.environ["LITELLM_RUST"] = "false"
litellm.suppress_debug_info = True

CONFIG = AmazonInvokeAgentConfig()
EMPTY: dict[str, JsonValue] = {}
MODEL_RESPONSE: TypeAdapter[ModelResponse] = TypeAdapter(ModelResponse)
JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL = "fixture-model"


class SuppliedFields(TypedDict):
    id: str
    created: int
    model: str | None
    object: str
    system_fingerprint: str | None
    service_tier: JsonValue
    trace: JsonValue


class TransformCase(TypedDict):
    status: int
    headers: list[tuple[str, str]]
    body: str
    supplied: SuppliedFields


class EventCase(TypedDict):
    event_type: str
    content_type: str
    message_type: str
    payload: JsonValue


class ContentCase(TypedDict):
    events: list[EventCase]


class Input(TypedDict, total=False):
    transform: list[TransformCase]
    content: list[ContentCase]


class MessageView(BaseModel):
    role: str
    content: str | None


class ChoiceView(BaseModel):
    index: int
    finish_reason: str | None
    message: MessageView


class UsageView(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseView(BaseModel):
    id: str
    object: str
    created: int
    model: str | None
    system_fingerprint: str | None
    choices: list[ChoiceView]
    usage: UsageView | None
    service_tier: JsonValue = None
    trace: JsonValue = None


VIEW: TypeAdapter[ResponseView] = TypeAdapter(ResponseView)


def _logging() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )


def _supplied(fields: SuppliedFields) -> ModelResponse:
    supplied: dict[str, JsonValue] = {
        "id": fields["id"],
        "created": fields["created"],
        "model": fields["model"],
        "object": fields["object"],
        "choices": [],
        "usage": None,
        "system_fingerprint": fields["system_fingerprint"],
        "service_tier": fields["service_tier"],
        "trace": fields["trace"],
    }
    return MODEL_RESPONSE.validate_python(supplied)


def _reply(case: TransformCase) -> httpx.Response:
    return httpx.Response(
        case["status"],
        headers=case["headers"],
        content=bytes.fromhex(case["body"]),
    )


def _transform(case: TransformCase) -> JsonValue:
    try:
        result = CONFIG.transform_response(
            model=MODEL,
            raw_response=_reply(case),
            model_response=_supplied(case["supplied"]),
            logging_obj=_logging(),
            request_data=EMPTY,
            messages=[],
            optional_params=EMPTY,
            litellm_params=EMPTY,
            encoding=None,
        )
    except BedrockError as error:
        return {"failure": {"status": error.status_code}}
    view = VIEW.validate_json(result.model_dump_json())
    return {"response": JSON_VALUE.validate_json(view.model_dump_json())}


def _event(case: EventCase) -> InvokeAgentEvent:
    headers: InvokeAgentEventHeaders = {
        "event_type": case["event_type"],
        "content_type": case["content_type"],
        "message_type": case["message_type"],
    }
    payload = cast(InvokeAgentEventPayload, case["payload"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/bedrock_invoke_agents.py#L93-L106
    return {"headers": headers, "payload": payload}


def _content(case: ContentCase) -> JsonValue:
    events = [_event(event) for event in case["events"]]
    try:
        content = CONFIG._extract_response_content(events)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_agent/transformation.py#L287-L308
    except (AttributeError, KeyError, TypeError):
        return {"failure": "structural"}
    return {"content": content}


def run(input_value: Input) -> JsonValue:
    if "transform" in input_value:
        return [_transform(case) for case in input_value["transform"]]
    if "content" in input_value:
        return [_content(case) for case in input_value["content"]]
    raise ValueError("unknown invoke_agent_reply operation")
