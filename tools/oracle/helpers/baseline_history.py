from typing import TypeAlias, cast

from litellm.proxy.spend_tracking.baseline_accounting import (
    BaselineEstimate,
    BaselineHistory,
    BaselineObservation,
    CacheEntry,
    advance_baseline_history,
)
from litellm.types.utils import Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROMPT_DETAIL_KEYS = ("text_tokens", "cached_tokens", "cache_creation_tokens")


def _cache_creation_token_details(details: dict[str, Json] | None) -> Json:
    if details is None:
        return None
    return {
        "ephemeral_5m_input_tokens": details.get("ephemeral_5m_input_tokens"),
        "ephemeral_1h_input_tokens": details.get("ephemeral_1h_input_tokens"),
    }


def _prompt_tokens_details(details: dict[str, Json] | None) -> Json:
    if details is None:
        return None
    result = {key: details.get(key) for key in _PROMPT_DETAIL_KEYS}
    creation_details = details.get("cache_creation_token_details")
    result["cache_creation_token_details"] = _cache_creation_token_details(
        cast(dict[str, Json] | None, creation_details)
    )
    if "cache_write_tokens" in details:
        result["cache_write_tokens"] = details["cache_write_tokens"]
    return result


def _usage_to_json(usage: Usage | None) -> Json:
    if usage is None:
        return None
    dump = usage.model_dump()
    result: dict[str, Json] = {
        "prompt_tokens": dump["prompt_tokens"],
        "completion_tokens": dump["completion_tokens"],
        "total_tokens": dump["total_tokens"],
        "prompt_tokens_details": _prompt_tokens_details(
            cast(dict[str, Json] | None, dump.get("prompt_tokens_details"))
        ),
    }
    for key in ("cache_read_input_tokens", "cache_creation_input_tokens"):
        if key in dump:
            result[key] = cast(Json, dump[key])
    return result


def _estimate_to_json(estimate: BaselineEstimate) -> Json:
    result: dict[str, Json] = {"request_id": estimate.request_id, "reason": estimate.reason}
    if estimate.provenance is not None:
        result["provenance"] = estimate.provenance
    if estimate.usage is not None:
        result["usage"] = _usage_to_json(estimate.usage)
    return result


def _entry_to_json(entry: CacheEntry) -> Json:
    return {
        "fingerprint": entry.fingerprint,
        "content_fingerprint": entry.content_fingerprint,
        "tokens": entry.tokens,
        "ttl_seconds": entry.ttl_seconds,
        "available_at": entry.available_at,
        "expires_at": entry.expires_at,
        "uncertain": entry.uncertain,
    }


def _history_to_json(history: BaselineHistory) -> Json:
    return {
        "first_at": history.first_at,
        "last_at": history.last_at,
        "equivalent": history.equivalent,
        "uncertain_before": history.uncertain_before,
        "entries": [_entry_to_json(entry) for entry in history.entries],
        "blocked_until": history.blocked_until,
    }


def _entry_from_json(entry: dict[str, Json]) -> CacheEntry:
    return CacheEntry(
        fingerprint=cast(str, entry["fingerprint"]),
        content_fingerprint=cast(str, entry["content_fingerprint"]),
        tokens=cast(int, entry["tokens"]),
        ttl_seconds=cast(int, entry["ttl_seconds"]),
        available_at=cast(float, entry["available_at"]),
        expires_at=cast(float, entry["expires_at"]),
        uncertain=cast(bool, entry.get("uncertain", False)),
    )


def _history_from_json(data: dict[str, Json] | None) -> BaselineHistory:
    if not data:
        return BaselineHistory()
    entries = data.get("entries", [])
    if not isinstance(entries, list):
        raise ValueError("history.entries must be an array")
    return BaselineHistory(
        first_at=cast(float | None, data.get("first_at")),
        last_at=cast(float | None, data.get("last_at")),
        equivalent=cast(bool, data.get("equivalent", True)),
        uncertain_before=cast(float, data.get("uncertain_before", 0.0)),
        entries=tuple(_entry_from_json(cast(dict[str, Json], entry)) for entry in entries),
        blocked_until=cast(float, data.get("blocked_until", 0.0)),
    )


def run(input_value: dict[str, Json]) -> Json:
    history = _history_from_json(cast(dict[str, Json] | None, input_value.get("history")))
    groups = input_value["groups"]
    if not isinstance(groups, list):
        raise ValueError("groups must be an array")
    steps: list[Json] = []
    for group in groups:
        if not isinstance(group, list):
            raise ValueError("each group must be an array")
        observations = [BaselineObservation.model_validate(item) for item in group]
        history, estimates = advance_baseline_history(history, observations)
        steps.append(
            {
                "estimates": [_estimate_to_json(estimate) for estimate in estimates],
                "history": _history_to_json(history),
            }
        )
    return {"steps": steps}
