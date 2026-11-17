from typing import Literal, TypedDict

from litellm.proxy.utils import PrismaClient, hash_token
from pydantic import JsonValue, TypeAdapter, ValidationError

MAPPING = TypeAdapter(dict[str, JsonValue])
HASHES = TypeAdapter(list[str])
VALUE = TypeAdapter(JsonValue)
TEXT = TypeAdapter(str)


class BlankClient(PrismaClient):
    # hash_token and jsonify_object read no state; the real __init__ builds a Prisma engine
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
        hashes = HASHES.validate_python([client.hash_token(token) for token in input_value["tokens"]])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L4617-L4621
        return MAPPING.validate_python({"hashes": hashes})
    hashes = HASHES.validate_python([hash_token(token) for token in input_value["tokens"]])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L7119-L7125
    return MAPPING.validate_python({"hashes": hashes})
