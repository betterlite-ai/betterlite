import io
from contextlib import redirect_stdout

import litellm
from pydantic import JsonValue

from .contracts import Call
from .values import (
    capability_report,
    reply_row,
    resource_row,
    transform_row,
    url_row,
    websocket_report,
)

litellm.suppress_debug_info = True


def run(input_value: Call) -> JsonValue:
    result: JsonValue
    with redirect_stdout(io.StringIO()):
        match input_value["function"]:
            case "base_and_api_version":
                result = {"steps": [url_row(step) for step in input_value["steps"]]}
            case "strip_model_prefix" | "reasoning_null_fields":
                result = {"steps": [transform_row(step) for step in input_value["steps"]]}
            case "websocket_model_query":
                result = websocket_report(input_value)
            case "resource_id_before_query":
                result = {"steps": [resource_row(step) for step in input_value["steps"]]}
            case "cancel_response":
                result = {"steps": [reply_row(step) for step in input_value["steps"]]}
            case "azure_responses_capabilities" | "o_series_options":
                result = capability_report(input_value)
    return result
