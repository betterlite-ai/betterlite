import json
from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail as Guardrail,
)
from litellm.types.llms.base import HiddenParams
from litellm.types.mcp import MCPPostCallResponseObject
from mcp.types import CallToolResult, EmbeddedResource, ImageContent, TextContent

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false
Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def guardrail() -> Guardrail:
    return Guardrail(api_key="key")


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def model_result(fields: dict[str, Json]) -> CallToolResult:
    items = cast(list[dict[str, Json]], fields.get("content", []))
    content: list[TextContent | ImageContent | EmbeddedResource] = [
        content_item(item) for item in items
    ]
    kwargs: dict[str, Json] = {"content": cast(Json, content)}
    if "structuredContent" in fields:
        kwargs["structuredContent"] = fields["structuredContent"]
    if "isError" in fields:
        kwargs["isError"] = fields["isError"]
    return CallToolResult(**kwargs)  # pyright: ignore[reportArgumentType]


def content_item(item: dict[str, Json]) -> TextContent | ImageContent:
    if item.get("type") == "image":
        return ImageContent(
            type="image",
            data=cast(str, item.get("data", "")),
            mime_type=cast(str, item.get("mimeType", "image/png")),
        )
    return TextContent(type="text", text=cast(str, item.get("text", "")))


def wrapped_result(fields: dict[str, Json]) -> MCPPostCallResponseObject:
    items = cast(list[dict[str, Json]], fields.get("mcp_tool_call_response", []))
    content: list[TextContent | ImageContent | EmbeddedResource] = [
        content_item(item) for item in items
    ]
    return MCPPostCallResponseObject(
        mcp_tool_call_response=content,
        hidden_params=HiddenParams(),
    )


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    result: Json
    match function:
        case "request_envelope":
            data = object_of(input_value["data"])
            result = cast(
                "Json",
                guardrail()._build_mcp_request_payload(data=data),  # pyright: ignore[reportUnknownMemberType]
            )
        case "response_envelope":
            request_data = object_of(input_value["request_data"])
            response = input_value["response"]
            result = cast(
                "Json",
                guardrail()._build_mcp_response_payload(  # pyright: ignore[reportUnknownMemberType]
                    request_data=request_data, response=response
                ),
            )
        case "hydrate_context":
            request_data = dict(object_of(input_value["request_data"]))
            Guardrail._hydrate_mcp_tool_context(
                cast("dict[str, object]", request_data)  # pyright: ignore[reportArgumentType]
            )
            result = cast("Json", request_data)
        case "normalize_dict":
            response = input_value["response"]
            result = cast("Json", Guardrail._normalize_mcp_response(response))
        case "normalize_model":
            fields = object_of(input_value["response"])
            result = cast("Json", Guardrail._normalize_mcp_response(model_result(fields)))
        case "normalize_field_tuples":
            pairs = cast("list[list[Json]]", input_value["pairs"])
            tuples = [tuple(pair) for pair in pairs]
            result = cast("Json", Guardrail._normalize_mcp_response(tuples))
        case "blocking_response":
            detail = input_value["detail"]
            response_obj = guardrail()._build_blocking_mcp_response(
                detail=detail, original_response_obj=None
            )
            dumped = response_obj.model_dump(exclude_none=True, by_alias=True)
            blocked_text = cast(TextContent, response_obj.mcp_tool_call_response[0]).text
            dumped["mcp_tool_call_response"][0]["text"] = json.loads(blocked_text)
            result = cast("Json", dumped)
        case "rewrite_text":
            response_obj = input_value["response"]
            text = cast(str, input_value["text"])
            replaced = Guardrail._set_mcp_tool_response_text(response_obj, text)
            result = {"replaced": replaced, "response": response_obj}
        case "rewrite_model_text" | "rewrite_field_tuples_text" | "rewrite_wrapped_text":
            result = rewrite_special(input_value, function)
        case _:
            raise ValueError(f"unknown function: {function}")
    return {"result": result}


def rewrite_special(input_value: dict[str, Json], function: str) -> Json:
    response_value = input_value["response"]
    text = cast(str, input_value["text"])
    if function == "rewrite_model_text":
        response_obj = model_result(object_of(response_value))
        replaced = Guardrail._set_mcp_tool_response_text(response_obj, text)
        response = response_obj.model_dump(
            exclude_none=True, by_alias=True, exclude={"result_type"}
        )
    elif function == "rewrite_wrapped_text":
        response_obj = wrapped_result(object_of(response_value))
        replaced = Guardrail._set_mcp_tool_response_text(response_obj, text)
        response = response_obj.model_dump(exclude_none=True, by_alias=True)
    else:
        pairs = cast(list[list[Json]], response_value)
        response_obj = [tuple(pair) for pair in pairs]
        replaced = Guardrail._set_mcp_tool_response_text(response_obj, text)
        response = response_obj
    return {"replaced": replaced, "response": cast("Json", response)}
