from decimal import Decimal
from typing import Any, TypeAlias

# The oracle receives JSON-shaped records; runtime guards below establish the mapping shapes.
# pyright: reportAny=false, reportUnknownVariableType=false, reportUnknownArgumentType=false, reportUnknownMemberType=false, reportOptionalIterable=false, reportGeneralTypeIssues=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _sum(old: int | None, new: int | None) -> int | None:
    if old is None and new is None:
        return None
    return (old or 0) + (new or 0)


def _cached(old: dict[str, Any] | None, new: dict[str, Any]) -> dict[str, Any]:
    names = ("text_tokens", "audio_tokens", "image_tokens")
    return {name: _sum(old.get(name) if old else None, new.get(name)) for name in names}


def _prompt(old: dict[str, Any] | None, new: dict[str, Any]) -> dict[str, Any]:
    result = dict(old or {})
    fields = (
        "text_tokens", "audio_tokens", "image_tokens", "video_tokens",
        "cache_write_tokens", "cache_creation_tokens",
    )
    for name in fields:
        if name == "cache_creation_tokens" and new.get("cache_write_tokens") is not None:
            continue
        if name in new:
            result[name] = _sum(result.get(name), new[name])
    if isinstance(new.get("cached_tokens_details"), dict):
        result["cached_tokens_details"] = _cached(
            result.get("cached_tokens_details"), new["cached_tokens_details"]
        )
    return result


def run(input_value: dict[str, Json]) -> Json:
    combined: dict[str, Any] = {"numeric_extras": {}}
    for usage in input_value.get("usages", []):
        for name in ("prompt_tokens", "completion_tokens", "total_tokens"):
            if name in usage:
                combined[name] = _sum(combined.get(name), usage[name])
        if isinstance(usage.get("prompt_tokens_details"), dict):
            combined["prompt_tokens_details"] = _prompt(
                combined.get("prompt_tokens_details"), usage["prompt_tokens_details"]
            )
        if isinstance(usage.get("completion_tokens_details"), dict):
            details = combined.setdefault("completion_tokens_details", {})
            names = (
                "text_tokens", "reasoning_tokens", "accepted_prediction_tokens",
                "rejected_prediction_tokens",
            )
            for name in names:
                if name in usage["completion_tokens_details"]:
                    details[name] = _sum(
                        details.get(name), usage["completion_tokens_details"][name]
                    )
        for name, value in usage.get("numeric_extras", {}).items():
            if name not in combined["numeric_extras"]:
                combined["numeric_extras"][name] = value
            else:
                combined["numeric_extras"][name] = (
                    Decimal(str(combined["numeric_extras"][name])) + Decimal(str(value))
                )
    return combined
