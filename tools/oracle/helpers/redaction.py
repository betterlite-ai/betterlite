import os
from collections.abc import Callable
from importlib import import_module
from typing import TypeAlias, cast

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

secret_redaction_module = import_module("litellm.litellm_core_utils.secret_redaction")

redact_string = cast(Callable[[str], str], secret_redaction_module.redact_string)
redact_internal_details = cast(
    Callable[[str], str], secret_redaction_module.redact_internal_details
)
redact_structured_value = cast(
    Callable[[str | None, str], str], secret_redaction_module.redact_structured_value
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def text(fields: dict[str, Json], name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def call(fields: dict[str, Json]) -> Json:
    match fields.get("function"):
        case "redact_string":
            return redact_string(text(fields, "text"))
        case "redact_internal_details":
            return redact_internal_details(text(fields, "text"))
        case "redact_structured_value":
            key = fields.get("key")
            if key is not None and not isinstance(key, str):
                raise ValueError("key must be a string or null")
            return redact_structured_value(key, text(fields, "value"))
        case other:
            raise ValueError(f"unknown redaction function: {other!r}")


def run(input_value: dict[str, Json]) -> Json:
    return {"result": call(input_value)}
