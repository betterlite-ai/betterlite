from collections.abc import Callable

import litellm
from pydantic import JsonValue

from .common import Input
from .cost import cost
from .embedding import endpoints
from .validate import api_base, redaction, user_agent, validate

litellm.suppress_debug_info = True

FUNCTIONS: dict[str, Callable[[Input], JsonValue]] = {
    "validate": validate,
    "api_base": api_base,
    "user_agent": user_agent,
    "redaction": redaction,
    "cost": cost,
    "endpoints": endpoints,
}


def run(input_value: Input) -> JsonValue:
    function = input_value.get("function")
    handler = None if function is None else FUNCTIONS.get(function)
    if handler is None:
        raise ValueError(f"unsupported b5980 function: {function}")
    return handler(input_value)
