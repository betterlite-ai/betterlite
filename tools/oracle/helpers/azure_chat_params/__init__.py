import io
from contextlib import redirect_stdout

from pydantic import JsonValue

from .configs import GPT5, O_SERIES_PARAMS
from .values import (
    alias_value,
    allowlist_value,
    install,
    map_value,
    stream_value,
    transform_value,
)
from .views import INPUT, Input


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed: Input = INPUT.validate_python(input_value)
    result: JsonValue
    with redirect_stdout(io.StringIO()):
        install(parsed["install"])
        if parsed["function"] == "gpt5_alias_catalog":
            result = {"models": [alias_value(step) for step in parsed["steps"]]}
        elif parsed["function"] == "native_stream":
            result = {"steps": [stream_value(step) for step in parsed["steps"]]}
        elif parsed["function"] == "parameter_mapping":
            result = {"steps": [map_value(step) for step in parsed["steps"]]}
        elif parsed["function"] == "request_transform":
            result = {"steps": [transform_value(step) for step in parsed["steps"]]}
        elif parsed["function"] == "gpt5_allowlist":
            result = {"models": [allowlist_value(GPT5, model) for model in parsed["models"]]}
        else:
            result = {
                "models": [allowlist_value(O_SERIES_PARAMS, model) for model in parsed["models"]]
            }
    return result
