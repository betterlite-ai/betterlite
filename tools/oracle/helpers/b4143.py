import asyncio
from collections.abc import Callable
from typing import Final, TypedDict

from litellm.proxy.common_utils import load_config_utils
from pydantic import JsonValue, TypeAdapter


class Operation(TypedDict):
    operation: str


class IncludeKeyInput(TypedDict):
    operation: str
    config_object_key: str
    include_entry: str


class LoadConfigInput(TypedDict):
    operation: str
    object_key: str
    objects: dict[str, JsonValue]


class Objects(dict[str, JsonValue]):
    def __call__(self, bucket_name: str) -> Callable[[str], JsonValue | None]:
        del bucket_name
        return self.read

    def read(self, object_key: str) -> JsonValue | None:
        return self.get(object_key)


OPERATION: Final = TypeAdapter(Operation)
INCLUDE_KEY: Final = TypeAdapter(IncludeKeyInput)
LOAD_CONFIG: Final = TypeAdapter(LoadConfigInput)
JSON_VALUE: Final = TypeAdapter(JsonValue)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    operation = OPERATION.validate_python(input_value)["operation"]
    if operation == "include_key":
        return _include_key(INCLUDE_KEY.validate_python(input_value))
    if operation == "load_config":
        return _load_config(LOAD_CONFIG.validate_python(input_value))
    raise ValueError(f"unknown b4143 operation: {operation}")


def _include_key(request: IncludeKeyInput) -> JsonValue:
    resolved = load_config_utils.resolve_include_object_key(
        request["config_object_key"], request["include_entry"]
    )
    result: dict[str, JsonValue] = {"key": resolved}
    return result


def _load_config(request: LoadConfigInput) -> JsonValue:
    load_config_utils.s3_object_reader = Objects(request["objects"])
    config = asyncio.run(
        load_config_utils.get_config_from_bucket(
            bucket_type=None,
            bucket_name="bucket",
            object_key=request["object_key"],
        )
    )
    result: dict[str, JsonValue] = {"config": JSON_VALUE.validate_python(config)}
    return result
