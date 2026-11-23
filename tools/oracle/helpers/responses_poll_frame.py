from typing import TypedDict

from litellm.proxy.response_polling.background_streaming import _sse_frame_data  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/response_polling/background_streaming.py#L78-L79
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Input(TypedDict):
    frames: list[str]


def frame_result(frame: str) -> JsonValue:
    payload = _sse_frame_data(frame)
    result: JsonValue = None
    if payload is None:
        result = {"kind": "no_data", "event": None}
    elif payload == "[DONE]":
        result = {"kind": "done", "event": None}
    else:
        try:
            event = JSON.validate_json(payload)
        except ValidationError:
            result = {"kind": "malformed", "event": None}
        else:
            result = {"kind": "event", "event": event}
    return result


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = [frame_result(frame) for frame in input_value["frames"]]
    output: JsonValue = {"results": results}
    return output
