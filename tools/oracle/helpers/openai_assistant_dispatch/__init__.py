from pydantic import JsonValue

from .assistants import assistants_result, threads_result
from .decisions import decisions_answer
from .realtime import realtime_answer
from .runs import runs_result
from .types import JSON, Input
from .wire import Wire


def wire_result(input_value: Input, wire: Wire) -> JsonValue:
    if input_value["kind"] == "assistants":
        return assistants_result(input_value, wire)
    if input_value["kind"] == "threads":
        return threads_result(input_value, wire)
    return runs_result(input_value, wire)


def run(input_value: Input) -> JsonValue:
    kind = input_value["kind"]
    requests: list[JsonValue] = []
    if kind == "decisions":
        result = decisions_answer(input_value)
    elif kind == "realtime":
        result = realtime_answer(input_value)
    else:
        wire = Wire(input_value["replies"])
        requests = wire.seen
        result = wire_result(input_value, wire)
    payload: dict[str, JsonValue] = {"requests": requests, "result": result}
    return JSON.validate_python(payload)
