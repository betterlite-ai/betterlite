from typing import NotRequired, TypedDict

import httpx
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

from .views import CHAT, logging_object
from .wire import MODEL, Object, ReplyCall


class UsageRecord(TypedDict, total=False):
    prompt_tokens: int | None
    completion_tokens: int | None
    total_tokens: int | None


class ToolFunctionRecord(TypedDict, total=False):
    name: str | None
    arguments: str | None


class ToolCallRecord(TypedDict, total=False):
    id: str | None
    type: str | None
    function: ToolFunctionRecord


class MessageRecord(TypedDict, total=False):
    content: JsonValue
    tool_calls: list[ToolCallRecord] | None


class ChoiceRecord(TypedDict, total=False):
    index: int | None
    finish_reason: str | None
    message: MessageRecord
    provider_specific_fields: Object | None


class ReplyRecord(TypedDict):
    choices: list[ChoiceRecord]
    id: NotRequired[str | None]
    object: NotRequired[str | None]
    created: NotRequired[int | None]
    model: NotRequired[str | None]
    usage: NotRequired[UsageRecord | None]


class HiddenCarrier(BaseModel):
    # the key set depends on the payload, so the hidden params stay open JSON
    params: Object = Field(alias="_hidden_params")


REPLY_RECORD: TypeAdapter[ReplyRecord] = TypeAdapter(ReplyRecord)


def usage_value(usage: UsageRecord | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def tool_call_value(call: ToolCallRecord) -> JsonValue:
    function = call.get("function")
    name = None if function is None else function.get("name")
    arguments = None if function is None else function.get("arguments")
    return {
        "id": call.get("id"),
        "type": call.get("type"),
        "function": {"name": name, "arguments": arguments},
    }


def tool_calls_value(calls: list[ToolCallRecord] | None) -> JsonValue:
    if calls is None:
        return None
    return [tool_call_value(call) for call in calls]


def choice_value(choice: ChoiceRecord) -> JsonValue:
    message = choice.get("message")
    native = choice.get("provider_specific_fields")
    content = None if message is None else message.get("content")
    calls = None if message is None else tool_calls_value(message.get("tool_calls"))
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "content": content,
        "tool_calls": calls,
        "native_finish_reason": None if native is None else native.get("native_finish_reason"),
    }


def reply_value(call: ReplyCall) -> JsonValue:
    request = httpx.Request("POST", "https://api.fireworks.ai/inference/v1/chat/completions")
    # content= would make httpx add its own Content-Length, and the case lists the upstream headers
    raw_response = httpx.Response(
        status_code=call["status"],
        headers=call["headers"],
        stream=httpx.ByteStream(call["body"].encode()),
        request=request,
    )
    # a streamed response has no text until it is read, and content= would put Content-Length in the headers
    raw_response.read()
    response: ModelResponse = CHAT.transform_response(
        model=MODEL,
        raw_response=raw_response,
        model_response=ModelResponse(),
        logging_obj=logging_object(),
        request_data={},
        messages=[],
        optional_params=call["optional_params"],
        litellm_params={},
        encoding=None,
        api_key=None,
        json_mode=None,
    )
    record = REPLY_RECORD.validate_json(response.model_dump_json())
    hidden = HiddenCarrier.model_validate(response, from_attributes=True).params
    return {
        "id": record.get("id"),
        "object": record.get("object"),
        "created": record.get("created"),
        "model": record.get("model"),
        "choices": [choice_value(choice) for choice in record["choices"]],
        "usage": usage_value(record.get("usage")),
        "hidden": hidden,
    }
