from collections.abc import Callable

from pydantic import JsonValue, TypeAdapter

from .image import image_body, image_params, image_reply, image_url
from .speech import (
    speech_body,
    speech_headers,
    speech_params,
    speech_reply,
    speech_url,
)
from .steps import Input, Step

HANDLERS: dict[str, Callable[[Step], JsonValue]] = {
    "speech_params": speech_params,
    "speech_body": speech_body,
    "speech_url": speech_url,
    "speech_headers": speech_headers,
    "speech_reply": speech_reply,
    "image_params": image_params,
    "image_url": image_url,
    "image_body": image_body,
    "image_reply": image_reply,
}


def run(value: dict[str, JsonValue]) -> JsonValue:
    request = TypeAdapter(Input).validate_python(value)
    return [HANDLERS[step["kind"]](step) for step in request["steps"]]
