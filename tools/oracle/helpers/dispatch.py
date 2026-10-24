import json
import os
import sys
from typing import TypeAlias, cast

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from provider_base_helpers import run as run_provider_base_helpers
from url_path import run as run_url_path

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
    if operation not in {"url_path", "provider_base_helpers"}:
        sys.stderr.write(f"unknown helper operation: {operation}\n")
        return 2
    try:
        if operation == "url_path":
            value = run_url_path(input_value)
        else:
            value = run_provider_base_helpers(input_value)
        output: dict[str, Json] = {"ok": True, "value": value}
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
