from datetime import UTC, datetime
from typing import Literal, TypedDict

import litellm
from litellm.a2a_protocol.cost_calculator import A2ACostCalculator
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._experimental.mcp_server.cost_calculator import MCPCostCalculator
from litellm.types.utils import Usage


class A2aInput(TypedDict):
    response_cost: float | None
    cost_per_query: float | None
    input_cost_per_token: float | None
    output_cost_per_token: float | None
    usage: Literal["absent", "mapping", "tokens"]
    prompt_tokens: int
    completion_tokens: int


class McpInput(TypedDict):
    response_cost: float | None
    tool_name: str
    default_cost_per_query: float | None
    tool_name_to_cost_per_query: dict[str, float]


class ReplicateInput(TypedDict):
    total_time: float


class A2aCase(TypedDict):
    function: Literal["a2a"]
    a2a: A2aInput | None


class McpCase(TypedDict):
    function: Literal["mcp"]
    mcp: McpInput | None


class ReplicateCase(TypedDict):
    function: Literal["replicate"]
    replicate_charge: ReplicateInput


Input = A2aCase | McpCase | ReplicateCase


def logging_record(call_type: str) -> Logging:
    return Logging(
        model="model",
        messages=None,
        stream=False,
        call_type=call_type,
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id="call",
        function_id="function",
    )


def agent_cost(agent: A2aInput | None) -> float:
    if agent is None:
        return A2ACostCalculator.calculate_a2a_cost(None)
    usage: dict[str, int] | Usage | None = None
    if agent["usage"] == "mapping":
        usage = {
            "prompt_tokens": agent["prompt_tokens"],
            "completion_tokens": agent["completion_tokens"],
        }
    if agent["usage"] == "tokens":
        usage = Usage(
            prompt_tokens=agent["prompt_tokens"],
            completion_tokens=agent["completion_tokens"],
        )
    prices = {
        "cost_per_query": agent["cost_per_query"],
        "input_cost_per_token": agent["input_cost_per_token"],
        "output_cost_per_token": agent["output_cost_per_token"],
    }
    record = logging_record("send_message")
    record.model_call_details = {
        "response_cost": agent["response_cost"],
        "litellm_params": prices,
        "usage": usage,
    }
    return A2ACostCalculator.calculate_a2a_cost(record)


def tool_cost(tool: McpInput | None) -> float:
    if tool is None:
        return MCPCostCalculator.calculate_mcp_tool_call_cost(None)
    cost_info = {
        "default_cost_per_query": tool["default_cost_per_query"],
        "tool_name_to_cost_per_query": tool["tool_name_to_cost_per_query"],
    }
    metadata = {"name": tool["tool_name"], "mcp_server_cost_info": cost_info}
    record = logging_record("call_mcp_tool")
    record.model_call_details = {
        "response_cost": tool["response_cost"],
        "mcp_tool_call_metadata": metadata,
    }
    return float(MCPCostCalculator.calculate_mcp_tool_call_cost(record))


def run(data: Input) -> dict[str, float]:
    if data["function"] == "a2a":
        return {"cost": agent_cost(data["a2a"])}
    if data["function"] == "mcp":
        return {"cost": tool_cost(data["mcp"])}
    return {
        "cost": litellm.completion_cost(
            model="replicate/unmapped-model",
            total_time=data["replicate_charge"]["total_time"],
        )
    }
