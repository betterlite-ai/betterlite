from collections.abc import Callable

import litellm
from pydantic import JsonValue

from .common import Input
from .headers import headers_value
from .messages import messages_value
from .params import params_value
from .response import response_value
from .stream import stream_value
from .url import url_value

litellm.suppress_debug_info = True

FUNCTIONS: dict[str, Callable[[Input], JsonValue]] = {
    "url": url_value,
    "headers": headers_value,
    "params": params_value,
    "messages": messages_value,
    "response": response_value,
    "stream": stream_value,
}


def run(input_value: Input) -> JsonValue:
    function = input_value.get("function")
    handler = None if function is None else FUNCTIONS.get(function)
    if handler is None:
        raise ValueError(f"unsupported b5841 function: {function}")
    return handler(input_value)
