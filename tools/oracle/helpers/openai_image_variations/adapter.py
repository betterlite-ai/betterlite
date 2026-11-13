from collections.abc import Callable
from typing import Final, TypedDict

import litellm
from pydantic import JsonValue as Json

from .probe import PROBE, Probe
from .request import request_value
from .transform import mapped_value, transform_value

litellm.suppress_debug_info = True


class Input(TypedDict):
    function: str
    probes: list[dict[str, Json]]


HANDLERS: Final[dict[str, Callable[[Probe], Json]]] = {
    "variation_image_part": transform_value,
    "n_size_format_user": mapped_value,
    "omitted_default_fields": request_value,
    "provider_error": request_value,
    "binary_file_descriptor": request_value,
    "variation_missing_config_data": request_value,
    "variation_reply_identity": request_value,
}


def run(input_value: Input) -> Json:
    handler = HANDLERS[input_value["function"]]
    return [handler(PROBE.validate_python(probe)) for probe in input_value["probes"]]
