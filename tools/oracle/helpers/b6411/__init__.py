import io
from contextlib import redirect_stdout

import litellm
from pydantic import JsonValue

from .contracts import INPUT, Input
from .values import (
    chat_request_value,
    extra_body_value,
    responses_params_value,
    supported_value,
    window_value,
)

litellm.suppress_debug_info = True


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed: Input = INPUT.validate_python(input_value)
    result: JsonValue
    with redirect_stdout(io.StringIO()):
        if parsed["function"] == "window":
            result = [window_value(step, parsed["model"]) for step in parsed["steps"]]
        elif parsed["function"] == "chat_request":
            result = [chat_request_value(step, parsed["model"]) for step in parsed["steps"]]
        elif parsed["function"] == "extra_body":
            result = [extra_body_value(step, parsed["model"]) for step in parsed["steps"]]
        elif parsed["function"] == "responses_params":
            result = [
                responses_params_value(step, parsed["model"]) for step in parsed["steps"]
            ]
        else:
            result = [supported_value(step) for step in parsed["steps"]]
    return result
