import json
from collections.abc import Mapping
from typing import TypeAlias, cast

from litellm.llms.anthropic.prompt_cache_prediction import (
    CacheBoundary,
    PromptPrefix,
    UnsupportedCachePlan,
    cache_scope,
    parse_cache_plan,
    parse_prompt,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SCOPE_FIELDS = ("caller_key_hash", "deployment_id", "provider_key", "model", "anthropic_version")


def body_value(input_value: dict[str, Json]) -> dict[str, Json]:
    text = input_value.get("body")
    if not isinstance(text, str):
        raise ValueError("body must be a JSON string")
    body = cast(Json, json.loads(text))
    if not isinstance(body, dict):
        raise ValueError("body must be a JSON object")
    return body


def plain(value: Mapping[str, object]) -> Json:
    return cast(Json, json.loads(json.dumps(value, default=dict)))


def boundary(value: CacheBoundary) -> Json:
    return {
        "fingerprint": value.fingerprint,
        "prefix_body": plain(value.prefix_body),
        "ttl_seconds": value.ttl_seconds,
        "lookback_fingerprints": list(value.lookback_fingerprints),
        "content_fingerprint": value.content_fingerprint,
        "lookback_content_fingerprints": list(value.lookback_content_fingerprints),
    }


def prompt(value: PromptPrefix | None) -> Json:
    if value is None:
        return None
    return {
        "prefix_body": plain(value.prefix_body),
        "fingerprint": value.fingerprint,
        "fingerprints": list(value.fingerprints),
        "ttl_seconds": value.ttl_seconds,
    }


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "cache_plan":
        plan = parse_cache_plan(body_value(input_value))
        if isinstance(plan, UnsupportedCachePlan):
            return {"unsupported": plan.reason}
        return {"full_body": plain(plan.full_body), "breakpoints": [boundary(item) for item in plan.breakpoints]}
    if function == "prompt":
        return {"prompt": prompt(parse_prompt(body_value(input_value)))}
    if function == "cache_scope":
        values = [input_value.get(name) for name in SCOPE_FIELDS]
        scope = [value for value in values if isinstance(value, str)]
        if len(scope) != len(SCOPE_FIELDS):
            raise ValueError("cache scope values must be strings")
        return cache_scope(*scope)
    raise ValueError(f"unknown anthropic cache plan function: {function}")
