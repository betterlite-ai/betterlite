from pydantic import JsonValue

from .handlers import run_call
from .types import CALLS


def run(raw: dict[str, JsonValue]) -> JsonValue:
    calls = CALLS.validate_python(raw["calls"])
    return [run_call(call) for call in calls]
