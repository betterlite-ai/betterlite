from typing import Literal, NotRequired, TypedDict

from litellm.responses.mcp.litellm_proxy_mcp_handler import LiteLLM_Proxy_MCP_Handler
from litellm.types.llms.openai import ResponsesAPIResponse
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import JsonValue, TypeAdapter

JSON = TypeAdapter(JsonValue)


class Call(TypedDict):
    function: Literal["store", "follow_up"]
    call_params: NotRequired[dict[str, JsonValue]]
    response: NotRequired[dict[str, JsonValue]]
    tool_results: NotRequired[list[dict[str, JsonValue]]]
    original_input: NotRequired[str | ResponseInputParam | None]
    preserve_reasoning: NotRequired[bool]


class Input(TypedDict):
    calls: list[Call]


def answer(call: Call) -> JsonValue:
    if call["function"] == "store":
        call_params = call.get("call_params")
        if call_params is None:
            raise ValueError("the store function needs call_params")
        return LiteLLM_Proxy_MCP_Handler._is_persistence_disabled(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1020
            call_params
        )
    response = call.get("response")
    if response is None:
        raise ValueError("the follow_up function needs response")
    tool_results = call.get("tool_results")
    if tool_results is None:
        raise ValueError("the follow_up function needs tool_results")
    preserve_reasoning = call.get("preserve_reasoning")
    if preserve_reasoning is None:
        raise ValueError("the follow_up function needs preserve_reasoning")
    follow_up = LiteLLM_Proxy_MCP_Handler._create_follow_up_input(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1025
        response=ResponsesAPIResponse.model_validate(response),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1388
        tool_results=tool_results,
        original_input=call.get("original_input"),
        preserve_reasoning=preserve_reasoning,
    )
    return JSON.validate_python(follow_up)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
