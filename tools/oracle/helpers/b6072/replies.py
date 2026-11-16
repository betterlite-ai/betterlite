from typing import NotRequired, TypedDict

import httpx
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

from .views import CHAT, logging_object
from .wire import MODEL, Object, ReplyCall


class UsageRecord(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ToolFunctionRecord(TypedDict):
    name: str | None
    arguments: str


class ToolCallRecord(TypedDict):
    id: str
    type: str
    function: ToolFunctionRecord


class MessageRecord(TypedDict):
    content: JsonValue
    tool_calls: list[ToolCallRecord] | None


class ChoiceRecord(TypedDict):
    index: int | None
    finish_reason: str | None
    message: MessageRecord
    provider_specific_fields: Object | None


class ReplyRecord(TypedDict):
    id: str | None
    object: str | None
    created: int | None
    model: str | None
    choices: list[ChoiceRecord]
    usage: NotRequired[UsageRecord | None]


class HiddenCarrier(BaseModel):
    # the key set depends on the payload, so the hidden params stay open JSON
    params: Object = Field(alias="_hidden_params")


REPLY_RECORD: TypeAdapter[ReplyRecord] = TypeAdapter(ReplyRecord)


def usage_value(usage: UsageRecord | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage["prompt_tokens"],
        "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def tool_calls_value(calls: list[ToolCallRecord] | None) -> JsonValue:
    if calls is None:
        return None
    values: list[JsonValue] = [
        {
            "id": call["id"],
            "type": call["type"],
            "function": {
                "name": call["function"]["name"],
                "arguments": call["function"]["arguments"],
            },
        }
        for call in calls
    ]
    return values


def choice_value(choice: ChoiceRecord) -> JsonValue:
    native = choice["provider_specific_fields"]
    return {
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "content": choice["message"]["content"],
        "tool_calls": tool_calls_value(choice["message"]["tool_calls"]),
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
    # a streamed response has no text until it is read
    raw_response.read()
    response: ModelResponse = CHAT.transform_response(
        MODEL,
        raw_response,
        ModelResponse(),
        logging_object(),
        {},
        [],
        call["optional_params"],
        {},
        None,
    )
    record = REPLY_RECORD.validate_json(response.model_dump_json())
    hidden = HiddenCarrier.model_validate(response, from_attributes=True).params
    return {
        "id": record["id"],
        "object": record["object"],
        "created": record["created"],
        "model": record["model"],
        "choices": [choice_value(choice) for choice in record["choices"]],
        "usage": usage_value(record.get("usage")),
        "hidden": hidden,
    }
