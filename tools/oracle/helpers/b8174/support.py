import os

from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from pydantic import JsonValue

Object = dict[str, JsonValue]


class Recorder(LiteLLMLoggingObj):
    def __init__(self) -> None:
        self.model_call_details: Object = {}


class UnsupportedOptionError(Exception):
    status_code: int

    def __init__(self, name: str, model: str) -> None:
        super().__init__(f"parameter {name} is not supported for model {model}")
        self.status_code = 400


def install_environment(values: dict[str, str | None]) -> None:
    os.environ.clear()
    for name, value in values.items():
        if value is not None:
            os.environ[name] = value


def object_or_empty(value: Object | None) -> Object:
    if value is None:
        return {}
    return value
