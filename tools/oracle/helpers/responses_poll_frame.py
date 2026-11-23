from typing import TypedDict

from litellm.proxy.response_polling.background_streaming import _sse_frame_data  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/response_polling/background_streaming.py#L78-L79
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Input(TypedDict):
    frames: list[str]


def frame_result(frame: str) -> JsonValue:
    payload = _sse_frame_data(frame)
    if payload is None:
        return {"kind": "no_data", "event": None}
    if payload == "[DONE]":
        return {"kind": "done", "event": None}
    try:
        event = JSON.validate_json(payload)
    except ValidationError:
        return {"kind": "malformed", "event": None}
    return {"kind": "event", "event": event}


def run(input_value: Input) -> JsonValue:
    results = [frame_result(frame) for frame in input_value["frames"]]
    return {"results": results}
