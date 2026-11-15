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
FIELDS: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])


# The typed dumps keep only the fields both sides compare, so a validated copy is the view.
def response_view(response: ModelResponse | TextCompletionResponse) -> Json:
    dump = DUMP.validate_json(response.model_dump_json())
    fields = FIELDS.validate_python(dump)
    usage = dump.get("usage")
    if dump.get("object") == TEXT_OBJECT:
        return clean_value({**fields, "kind": "text", "usage": usage_view(usage, priced=False)})
    chat = {**fields, "kind": "chat", "usage": usage_view(usage, priced=True)}
    return clean_value({**chat, "hidden_params": hidden_view(response)})


# model_dump_json leaves the private _hidden_params map out, so it is read as an attribute.
def hidden_view(response: ModelResponse | TextCompletionResponse) -> Json:
    hidden = PrivateDump.model_validate(response).hidden
    cost = hidden.get("response_cost")
    return {
        "response_cost": None if cost is None else f"{cost:.12f}",
        "provider_specific_fields": hidden.get("provider_specific_fields"),
    }


def usage_view(usage: UsageDump | None, *, priced: bool) -> Json:
    if usage is None:
        return None
    view: dict[str, Json] = {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }
    cost = usage.get("cost")
    if priced and cost is not None:
        view["cost"] = f"{cost:.12f}"
    return view


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
