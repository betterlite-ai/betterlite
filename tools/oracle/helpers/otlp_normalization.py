import json
from typing import TypeAlias, TypedDict, cast

from litellm.tracing import messages, ui_format
from litellm.tracing.decode import (
    _truncate,  # pyright: ignore[reportPrivateUsage]
    _truncate_payload,  # pyright: ignore[reportPrivateUsage]
    encode_otlp_response,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Repeat(TypedDict):
    text: str
    times: int


Piece: TypeAlias = str | Repeat


class Response(TypedDict):
    content_type: str | None
    error: str | None


def joined(pieces: list[Piece]) -> str:
    # Cases repeat a piece so a 64 KiB value stays a short line in the case file
    return "".join(p if isinstance(p, str) else p["text"] * p["times"] for p in pieces)


def run(input_value: dict[str, dict[str, Json]]) -> Json:
    operation, body = next(iter(input_value.items()))
    if operation == "response_encoding":
        response = cast(Response, cast(object, body))
        encoded, media_type = encode_otlp_response(response["content_type"], response["error"])
        return {"body": list(encoded), "content_type": media_type}
    if operation == "content_text":
        return messages.content_text(cast(Json, json.loads(cast(str, body["content"]))))
    value = joined(cast(list[Piece], body["value"]))
    if operation == "truncate":
        return _truncate(value)
    if operation == "elision":
        return _truncate_payload(value)
    return cast(Json, json.loads(json.dumps(ui_format.to_ui_content(value))))
