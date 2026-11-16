from typing import Literal, TypedDict

import litellm
from pydantic import JsonValue as Json

from .exchange import exchange
from .native import native_audio_config, native_config, native_request, native_response
from .probe import PROBE

litellm.suppress_debug_info = True


class Input(TypedDict):
    function: Literal["exchange", "native_config", "native_response", "native_request"]
    probes: list[dict[str, Json]]


def run(input_value: Input) -> Json:
    values: list[Json] = []
    for raw in input_value["probes"]:
        probe = PROBE.validate_python(raw)
        match input_value["function"]:
            case "exchange":
                values.append(exchange(probe))
            case "native_config":
                values.append(native_config(native_audio_config(), probe))
            case "native_response":
                values.append(native_response(native_audio_config(), probe))
            case "native_request":
                values.append(native_request(native_audio_config(), probe))
    return values
