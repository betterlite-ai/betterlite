from typing import TypeAlias, cast

from litellm.llms.sap.chat.models import (
    ChatCompletionTool,
    MaskingModuleConfig,
    OrchestrationRequest,
    ResponseFormat,
    ResponseFormatJSONSchema,
    SAPAssistantMessage,
    SAPMessage,
    SAPToolChatMessage,
    SAPUserMessage,
)
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

MESSAGE_MODELS: dict[str, type[BaseModel]] = {
    "user": SAPUserMessage,
    "assistant": SAPAssistantMessage,
    "tool": SAPToolChatMessage,
}


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def dump(data: dict[str, Json], model: type[BaseModel]) -> Json:
    return cast(Json, model.model_validate(data).model_dump(by_alias=True, exclude_unset=True))


def dispatch(model: str, data: dict[str, Json]) -> Json:
    if model == "orchestration_request":
        return dump(data, OrchestrationRequest)
    if model == "chat_completion_tool":
        return dump(data, ChatCompletionTool)
    if model == "masking_module":
        return dump(data, MaskingModuleConfig)
    if model == "response_format":
        if data.get("type") == "json_schema":
            return dump(data, ResponseFormatJSONSchema)
        return dump(data, ResponseFormat)
    if model == "sap_message":
        role = data.get("role")
        selected = MESSAGE_MODELS.get(role if isinstance(role, str) else "", SAPMessage)
        return dump(data, selected)
    raise ValueError(f"unknown model {model}")


def one(call: Json) -> Json:
    fields = object_value(call, "call")
    model = string_value(fields.get("model"), "model")
    data = object_value(fields.get("data"), "data")
    try:
        value = dispatch(model, data)
    except ValueError:
        return {"ok": False}
    return {"ok": True, "value": value}


def run(input_value: dict[str, Json]) -> Json:
    return [one(call) for call in array_value(input_value.get("calls"), "calls")]
