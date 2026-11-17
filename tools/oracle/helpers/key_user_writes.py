from typing import Literal, TypedDict

from litellm.proxy.utils import PrismaClient, hash_token
from pydantic import JsonValue, TypeAdapter, ValidationError

MAPPING = TypeAdapter(dict[str, JsonValue])
VALUE = TypeAdapter(JsonValue)
TEXT = TypeAdapter(str)


class BlankClient(PrismaClient):
    # hash_token and jsonify_object read no instance state, and the real __init__ builds a Prisma engine
    def __init__(self) -> None:
        pass


class HashInput(TypedDict):
    function: Literal["class_hash", "module_hash"]
    tokens: list[str]


class JsonInput(TypedDict):
    function: Literal["class_json"]
    fields: dict[str, JsonValue]


Input = HashInput | JsonInput


def run(input_value: Input) -> JsonValue:
    client = BlankClient()
    if input_value["function"] == "class_json":
        original = input_value["fields"]
        converted = client.jsonify_object(dict(original))
        reported: dict[str, JsonValue] = {}
        for name, value in original.items():
            try:
                MAPPING.validate_python(value)
            except ValidationError:
                plain = VALUE.validate_python(converted[name])
                reported[name] = {"kind": "plain", "value": plain}
            else:
                encoded = TEXT.validate_python(converted[name])
                parsed = VALUE.validate_json(encoded)
                reported[name] = {"kind": "encoded", "value": parsed}
        return {"fields": reported}
    if input_value["function"] == "class_hash":
        return {"hashes": [client.hash_token(token) for token in input_value["tokens"]]}
    return {"hashes": [hash_token(token) for token in input_value["tokens"]]}
