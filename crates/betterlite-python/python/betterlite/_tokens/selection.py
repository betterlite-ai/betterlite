import json

from betterlite import _native


# ast-grep-ignore: no-one-line-functions
def openai_tokenizer_encoding_name(model: str) -> str:
    return _native.openai_encoding_name(json.dumps({"model": model}))


# ast-grep-ignore: no-one-line-functions
def uses_legacy_message_accounting(model: str) -> bool:
    return _native.legacy_message_accounting(json.dumps({"model": model}))
