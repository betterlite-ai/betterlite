from typing import Final, TypedDict

from litellm.types.utils import ModelResponse, TextCompletionResponse
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json

TEXT_OBJECT = "text_completion"


class MessageDump(TypedDict, total=False):
    content: Json
    role: Json
    tool_calls: Json
    function_call: Json
    provider_specific_fields: Json
    reasoning_content: Json
    thinking_blocks: Json
    annotations: Json
    audio: Json
    images: Json


class ChoiceDump(TypedDict, total=False):
    index: Json
    finish_reason: Json
    message: MessageDump
    text: Json
    logprobs: Json
    provider_specific_fields: Json


class UsageDump(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cost: float | None


class HiddenDump(TypedDict, total=False):
    response_cost: float | None
    provider_specific_fields: Json


class PrivateDump(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    hidden: HiddenDump = Field(alias="_hidden_params")


class ResponseDump(TypedDict, total=False):
    id: Json
    object: Json
    created: Json
    model: Json
    system_fingerprint: Json
    service_tier: Json
    choices: list[ChoiceDump]
    usage: UsageDump | None


DUMP: Final[TypeAdapter[ResponseDump]] = TypeAdapter(ResponseDump)


def response_view(response: ModelResponse | TextCompletionResponse) -> Json:
    dump = DUMP.validate_json(response.model_dump_json())
    if dump.get("object") == TEXT_OBJECT:
        return clean_value(text_view(dump))
    return clean_value(chat_view(dump, hidden_view(response)))


# model_dump_json leaves the private _hidden_params map out, so it is read as an attribute.
def hidden_view(response: ModelResponse | TextCompletionResponse) -> Json:
    hidden = PrivateDump.model_validate(response).hidden
    cost = hidden.get("response_cost")
    return {
        "response_cost": None if cost is None else f"{cost:.12f}",
        "provider_specific_fields": hidden.get("provider_specific_fields"),
    }


def chat_view(dump: ResponseDump, hidden: Json) -> Json:
    return {
        "kind": "chat",
        "id": dump.get("id"),
        "object": dump.get("object"),
        "created": dump.get("created"),
        "model": dump.get("model"),
        "system_fingerprint": dump.get("system_fingerprint"),
        "service_tier": dump.get("service_tier"),
        "choices": chat_choices(dump.get("choices")),
        "usage": usage_view(dump.get("usage")),
        "hidden_params": hidden,
    }


def chat_choices(choices: list[ChoiceDump] | None) -> Json:
    if choices is None:
        return []
    return [chat_choice(choice) for choice in choices]


def chat_choice(choice: ChoiceDump) -> Json:
    message = choice.get("message")
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "provider_specific_fields": choice.get("provider_specific_fields"),
        "message": None if message is None else message_view(message),
    }


def message_view(message: MessageDump) -> Json:
    return {
        "content": message.get("content"),
        "role": message.get("role"),
        "tool_calls": message.get("tool_calls"),
        "function_call": message.get("function_call"),
        "provider_specific_fields": message.get("provider_specific_fields"),
        "reasoning_content": message.get("reasoning_content"),
        "thinking_blocks": message.get("thinking_blocks"),
        "annotations": message.get("annotations"),
        "audio": message.get("audio"),
        "images": message.get("images"),
    }


def usage_view(usage: UsageDump | None) -> Json:
    if usage is None:
        return None
    cost = usage.get("cost")
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
        "cost": None if cost is None else f"{cost:.12f}",
    }


def text_view(dump: ResponseDump) -> Json:
    return {
        "kind": "text",
        "id": dump.get("id"),
        "object": dump.get("object"),
        "created": dump.get("created"),
        "model": dump.get("model"),
        "system_fingerprint": dump.get("system_fingerprint"),
        "choices": text_choices(dump.get("choices")),
        "usage": text_usage_view(dump.get("usage")),
    }


def text_choices(choices: list[ChoiceDump] | None) -> Json:
    if choices is None:
        return []
    return [text_choice(choice) for choice in choices]


def text_choice(choice: ChoiceDump) -> Json:
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "text": choice.get("text"),
        "logprobs": choice.get("logprobs"),
    }


def text_usage_view(usage: UsageDump | None) -> Json:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def clean_value(value: Json) -> Json | None:
    match value:
        case dict() as fields:
            return clean_fields(fields)
        case list() as items:
            return [kept for item in items if (kept := clean_value(item)) is not None]
        case None | bool() | int() | float() | str():
            return value


def clean_fields(fields: dict[str, Json]) -> dict[str, Json]:
    cleaned: dict[str, Json] = {}
    for name, item in fields.items():
        kept = clean_value(item)
        if kept is not None:
            cleaned[name] = kept
    return cleaned
