import time
from collections.abc import Callable
from functools import partial
from typing import Final, TypedDict

import litellm
from pydantic import JsonValue as Json

from .probe import NOW, PROBE, Probe
from .request import request_value
from .transform import mapped_value, transform_value

litellm.suppress_debug_info = True


class Input(TypedDict):
    function: str
    probes: list[dict[str, Json]]


HANDLERS: Final[dict[str, Callable[[Probe], Json]]] = {
    "dalle2_parameters": transform_value,
    "dalle3_quality_style": transform_value,
    "gpt_image_usage": transform_value,
    "b64_json_and_url": transform_value,
    "missing_created": transform_value,
    "unsupported_image_param": mapped_value,
    "image_status_error": request_value,
    "image_request_overlay": request_value,
    "image_raw_response_headers": request_value,
}


def run(input_value: Input) -> Json:
    time.time = partial(float, NOW)
    handler = HANDLERS[input_value["function"]]
    return [handler(PROBE.validate_python(probe)) for probe in input_value["probes"]]
