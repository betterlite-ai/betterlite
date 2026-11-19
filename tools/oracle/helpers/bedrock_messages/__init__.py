import asyncio

import litellm
from pydantic import JsonValue

from .calls import constructor, drain, error_class, transform
from .transport import invoke_url
from .views import Input

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "invoke_url":
            return invoke_url(input_value)
        case "transform":
            return transform(input_value)
        case "event_stream":
            return asyncio.run(drain(input_value))
        case "error_class":
            return error_class(input_value)
        case "constructor":
            return constructor(input_value)
