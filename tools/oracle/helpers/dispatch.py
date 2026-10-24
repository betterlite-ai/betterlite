import json
import os
import sys
from typing import TYPE_CHECKING, TypeAlias, cast

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from dot_paths import run as run_dot_paths
from url_path import run as run_url_path

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def parse_input(text: str) -> dict[str, Json]:
    value: Json = cast(Json, json.loads(text))
    return object_value(value, "request")


def main() -> int:
    try:
        request = parse_input(sys.stdin.read(1024 * 1024 + 1))
        if len(json.dumps(request)) > 1024 * 1024:
            raise ValueError("request exceeds 1 MiB")
        operation = string_value(request.get("operation"), "operation")
        input_value = object_value(request.get("input"), "input")
    except Exception as error:  # noqa: BLE001 - malformed protocol input is reported on stderr
        sys.stderr.write(f"invalid helper request: {error}\n")
        return 2
    runner: Callable[[dict[str, Json]], object]
    match operation:
        case "url_path":
            runner = run_url_path
        case "dot_paths":
            runner = run_dot_paths
        case _:
            sys.stderr.write(f"unknown helper operation: {operation}\n")
            return 2
    try:
        value = runner(input_value)
        output: dict[str, object] = {"ok": True, "value": value}
    except Exception as error:  # noqa: BLE001 - the wire contract names every helper exception
        output = {
            "ok": False,
            "error": {
                "class": type(error).__name__,
                "status": None,
                "type": None,
                "param": None,
                "code": None,
                "message": str(error),
            },
        }
    sys.stdout.write(json.dumps(output, separators=(",", ":")))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
