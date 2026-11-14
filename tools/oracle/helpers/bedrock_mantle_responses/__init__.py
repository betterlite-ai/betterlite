import io
import os
from collections.abc import Callable
from contextlib import redirect_stdout
from functools import partial
from typing import Final

from pydantic import JsonValue

from .handlers import auth_cases, hoist_cases, map_cases, url_cases

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

HANDLERS: Final[dict[str, Callable[[JsonValue], JsonValue]]] = {
    "mantle_responses_url": url_cases,
    "mantle_response_options": partial(map_cases, with_body=False),
    "mantle_model_name": partial(map_cases, with_body=True),
    "unsupported_params_400": partial(map_cases, with_body=False),
    "mantle_response_auth": auth_cases,
    "unsupported_params_warn_drop": partial(map_cases, with_body=False),
    "openai_project_header": auth_cases,
    "mantle_tool_filter": partial(map_cases, with_body=False),
    "mantle_input_hoisting": hoist_cases,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("bedrock_mantle_responses takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    # LiteLLM logs below WARNING to stdout, which carries the helper's JSON answer
    with redirect_stdout(io.StringIO()):
        return handler(payload)
