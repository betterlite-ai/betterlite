import sys
from importlib import import_module
from pathlib import Path
from typing import Protocol, TypeAlias, cast

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

__path__ = [str(Path(__file__).with_suffix(""))]

SERVICE_MODULES = {
    "native_responses_relay": "websocket_oracle.native_responses_relay",
    "managed_responses_turns": "websocket_oracle.managed_responses_turns",
    "realtime_probe": "websocket_oracle.realtime_probe",
}


class ServiceAdapter(Protocol):
    def run(self, input_value: dict[str, Json]) -> Json: ...


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def run(input_value: dict[str, Json]) -> Json:
    if sys.modules.get("litellm") is not litellm:
        raise ValueError("LiteLLM module is unavailable")
    if set(input_value) != {"service", "input"}:
        raise ValueError("websocket input must contain service and input")
    service = string_value(input_value["service"], "service")
    module_name = SERVICE_MODULES.get(service)
    if module_name is None:
        raise ValueError(f"unknown websocket service: {service}")
    adapter = cast(ServiceAdapter, import_module(module_name))
    return adapter.run(object_value(input_value["input"], "input"))
