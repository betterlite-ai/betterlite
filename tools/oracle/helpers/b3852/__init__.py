import litellm
from pydantic import JsonValue as Json

from .scenarios import (
    run_cache_eviction,
    run_close_dedup,
    run_mcp_tool_call,
    run_metric_error,
    run_pre_call,
    run_project_headers,
)
from .wire import Input

litellm.suppress_debug_info = True


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "pre_call":
        return run_pre_call(input_value)
    if function == "mcp_tool_call":
        return run_mcp_tool_call(input_value)
    if function == "metric_error":
        return run_metric_error(input_value)
    if function == "close_dedup":
        return run_close_dedup(input_value)
    if function == "cache_eviction":
        return run_cache_eviction(input_value)
    return run_project_headers(input_value)
