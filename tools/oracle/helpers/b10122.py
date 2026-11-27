from typing import Final, Literal, TypeAlias, TypedDict, cast

from litellm.responses.mcp.litellm_proxy_mcp_handler import (
    LiteLLM_Proxy_MCP_Handler,
)
from litellm.types.llms.openai import ResponsesAPIResponse
from openai.types.responses.response_output_item import ResponseOutputItem
from pydantic import JsonValue, TypeAdapter

RESPONSE: Final[TypeAdapter[ResponsesAPIResponse]] = TypeAdapter(ResponsesAPIResponse)


class AutoExecuteCall(TypedDict):
    function: Literal["should_auto_execute_tools"]
    references: list[dict[str, JsonValue]]


class ResponseCall(TypedDict):
    function: Literal["extract_tool_calls_from_response"]
    response: dict[str, JsonValue]


class OutputToolCall(TypedDict):
    kind: Literal["output"]
    item: dict[str, JsonValue]


class MappingToolCall(TypedDict):
    kind: Literal["mapping"]
    fields: dict[str, JsonValue]


class DetailsCall(TypedDict):
    function: Literal["extract_tool_call_details"]
    tool_call: OutputToolCall | MappingToolCall


Call: TypeAlias = AutoExecuteCall | ResponseCall | DetailsCall


class Input(TypedDict):
    calls: list[Call]


def run(input_value: Input) -> list[JsonValue]:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return answers


def answer(call: Call) -> JsonValue:
    if call["function"] == "should_auto_execute_tools":
        return LiteLLM_Proxy_MCP_Handler._should_auto_execute_tools(call["references"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L539-L563
    if call["function"] == "extract_tool_calls_from_response":
        return extracted_values(call["response"])
    return details_answer(call["tool_call"])


def extracted_values(response: dict[str, JsonValue]) -> list[JsonValue]:
    decoded = RESPONSE.validate_python(response)
    calls = cast(list[ResponseOutputItem | dict[str, JsonValue]], LiteLLM_Proxy_MCP_Handler._extract_tool_calls_from_response(decoded))  # noqa: E501, SLF001  # pyright: ignore[reportAny, reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L565-L577
    values: list[JsonValue] = [details_value(tool_call) for tool_call in calls]
    return values


def details_answer(tool_call: OutputToolCall | MappingToolCall) -> JsonValue:
    if tool_call["kind"] == "mapping":
        return details_value(tool_call["fields"])
    body: dict[str, JsonValue] = {
        "id": "resp_tool_selection",
        "created_at": 1,
        "output": [tool_call["item"]],
    }
    decoded = RESPONSE.validate_python(body)
    return details_value(cast(ResponseOutputItem | dict[str, JsonValue], decoded.output[0]))  # type: ignore[misc]  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L603-L635


def details_value(tool_call: ResponseOutputItem | dict[str, JsonValue]) -> JsonValue:
    name, arguments, call_id = LiteLLM_Proxy_MCP_Handler._extract_tool_call_details(tool_call)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L603-L635
    view: list[JsonValue] = [name, arguments, call_id]
    return view
