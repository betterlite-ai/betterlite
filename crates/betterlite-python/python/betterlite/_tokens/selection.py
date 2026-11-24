import json

from pydantic import JsonValue

from betterlite import _native


def openai_tokenizer_encoding_name(model: str) -> str:
    request: dict[str, JsonValue] = {"model": model}
    return _native.openai_encoding_name(json.dumps(request))


def uses_legacy_message_accounting(model: str) -> bool:
    request: dict[str, JsonValue] = {"model": model}
    return _native.legacy_message_accounting(json.dumps(request))
