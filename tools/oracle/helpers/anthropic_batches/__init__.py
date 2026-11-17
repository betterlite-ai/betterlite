from collections.abc import Callable
from typing import Final

import litellm
from pydantic import JsonValue

from .auth import auth_headers
from .lifecycle import create_requests, error_counts, lifecycle
from .results import result_response
from .retrieve import retrieve

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


HANDLERS: Final[dict[str, Callable[[JsonValue], JsonValue]]] = {
    "auth_headers": auth_headers,
    "create_requests": create_requests,
    "error_counts": error_counts,
    "lifecycle": lifecycle,
    "result_response": result_response,
    "retrieve": retrieve,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown anthropic_batches variant: {name}")
    return handler(input_value[name])
