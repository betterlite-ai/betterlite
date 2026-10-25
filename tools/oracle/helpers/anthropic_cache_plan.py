import dataclasses
import json
from collections.abc import Mapping
from typing import TypeAlias, cast

from litellm.llms.anthropic.prompt_cache_prediction import (
    UnsupportedCachePlan,
    cache_scope,
    parse_cache_plan,
    parse_prompt,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def json_value(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(plain(value), separators=(",", ":"), ensure_ascii=False)))


def plain(value: object) -> object:
    if dataclasses.is_dataclass(value):
        return {field.name: plain(getattr(value, field.name)) for field in dataclasses.fields(value)}
    if isinstance(value, Mapping):
        return {key: plain(item) for key, item in value.items()}
    if isinstance(value, (tuple, list)):
        return [plain(item) for item in value]
    return value


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "cache_plan":
        body = input_value.get("body")
        result = parse_cache_plan(body if isinstance(body, dict) else {})
        if isinstance(result, UnsupportedCachePlan):
            return {"kind": "unsupported", "reason": result.reason}
        return {"kind": "plan", **json_value(result)}
    if function == "prompt":
        body = input_value.get("body")
        result = parse_prompt(body if isinstance(body, dict) else {})
        return None if result is None else {"kind": "prompt", **json_value(result)}
    if function == "cache_scope":
        values = [input_value.get(name, "") for name in ("caller_key_hash", "deployment_id", "provider_key", "model")]
        values.append(input_value.get("anthropic_version", "2023-06-01"))
        if not all(isinstance(value, str) for value in values):
            raise ValueError("cache scope values must be strings")
        return {"kind": "scope", "fingerprint": cache_scope(*values)}
    raise ValueError(f"unknown anthropic cache plan function: {function}")
