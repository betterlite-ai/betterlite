from typing import TypedDict

from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue, TypeAdapter, ValidationError

DUMP = TypeAdapter(dict[str, JsonValue])


class JsonCall(TypedDict):
    name: str
    json: str


class ObjectCall(TypedDict):
    name: str
    object: dict[str, JsonValue]


class Input(TypedDict):
    json_strings: list[JsonCall]
    objects: list[ObjectCall]


def run(input_value: Input) -> dict[str, JsonValue]:
    calls: list[JsonValue] = []
    for json_call in input_value["json_strings"]:
        try:
            batch = LiteLLMBatch.model_validate_json(json_call["json"])
        except ValidationError:
            calls.append({"name": json_call["name"], "ok": False})
            continue
        dumped = DUMP.validate_json(batch.model_dump_json())
        calls.append({"name": json_call["name"], "ok": True, "batch": dumped})
    for object_call in input_value["objects"]:
        try:
            batch = LiteLLMBatch.model_validate(object_call["object"])
        except ValidationError:
            calls.append({"name": object_call["name"], "ok": False})
            continue
        dumped = DUMP.validate_json(batch.model_dump_json())
        calls.append({"name": object_call["name"], "ok": True, "batch": dumped})
    return {"calls": calls}
