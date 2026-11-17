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


def field_view(original: JsonValue, converted: object) -> dict[str, JsonValue]:
    try:
        MAPPING.validate_python(original)
    except ValidationError:
        return {"kind": "plain", "value": VALUE.validate_python(converted)}
    encoded = TEXT.validate_python(converted)
    return {"kind": "encoded", "value": VALUE.validate_json(encoded)}


def run(input_value: Input) -> JsonValue:
    client = BlankClient()
    if input_value["function"] == "class_json":
        original = input_value["fields"]
        converted = client.jsonify_object(dict(original))
        reported = {name: field_view(value, converted[name]) for name, value in original.items()}
        return {"fields": reported}
    if input_value["function"] == "class_hash":
        return {"hashes": [client.hash_token(token) for token in input_value["tokens"]]}
    return {"hashes": [hash_token(token) for token in input_value["tokens"]]}
