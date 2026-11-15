from litellm.litellm_core_utils.llm_cost_calc.tool_call_cost_tracking import (
    StandardBuiltInToolCostTracking as ToolCostTracking,
)
from litellm.types.utils import ModelResponse, ServerToolUse, Usage
from pydantic import JsonValue

from .wire import Call, Object


def cost_value(call: Call) -> JsonValue:
    usage = Usage(
        server_tool_use=ServerToolUse(
            web_search_requests=call.get("web_search_requests"),
            browser_open_requests=call.get("browser_open_requests"),
        )
    )
    # the public entry resolves the model's model_info and calls cost_per_web_search_request
    cost = ToolCostTracking.get_cost_for_built_in_tools(
        model=call["model"],
        response_object=ModelResponse(),
        usage=usage,
        custom_llm_provider="groq",
    )
    value: Object = {"cost": format(cost, ".12f")}
    return value
