import base64
from typing import Final

import litellm
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

MODEL: Final = "sora-2"
TIMEOUT: Final = 60
SPEECH_MAX_RETRIES: Final = 0

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
OBJECTS: Final = TypeAdapter(list[JsonValue])
TEXT: Final = TypeAdapter(str)
BYTES: Final = TypeAdapter(bytes)


def required(value: str | None, name: str) -> str:
    if value is None:
        raise ValueError(f"the case needs {name}")
    return value


def leaf_parts(files: list[JsonValue]) -> list[JsonValue]:
    parts: list[JsonValue] = []
    for part in files:
        fields = OBJECT.validate_python(part)
        body = TEXT.validate_python(fields["body"])
        if fields["body_kind"] == "bytes":
            body = base64.b64decode(body).decode()
        parts.append(
            {
                "field": fields["field"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": body,
            }
        )
    return parts
