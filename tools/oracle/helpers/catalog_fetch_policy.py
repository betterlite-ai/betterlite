import asyncio
import random
from collections.abc import Awaitable, Callable, Coroutine
from decimal import Decimal
from typing import Protocol, TypeAlias, cast

import httpx
import litellm.litellm_core_utils.get_model_cost_map as cost_map
from litellm.litellm_core_utils.get_model_cost_map import (
    GetModelCostMap,
    ModelCostMapReloaded,
    ModelCostMapReloadUnavailable,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CostMap: TypeAlias = dict[str, dict[str, Json]]
Headers: TypeAlias = dict[str, str]

git_blob_id = cast(Callable[[bytes], str], cost_map.git_blob_id)
count_entries = cast(Callable[[CostMap], int], cost_map._count_model_entries)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
classify_response = cast(
    "Callable[[httpx.Response, str], object]",
    cost_map._classify_fetch_response,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
)
parse_retry_after = cast(
    "Callable[[httpx.Response], float | None]",
    cost_map._parse_retry_after_seconds,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
)
retry_wait = cast(
    "Callable[[object, int, random.Random], float]",
    cost_map._retry_wait_seconds,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
)
next_retry_wait = cast(
    "Callable[[object, int, int, random.Random], float | object]",
    cost_map._next_retry_wait,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
)
class _FetchCall(Protocol):
    def __call__(  # noqa: PLR0913
        self,
        *,
        url: str,
        timeout: int,
        max_attempts: int,
        sleep: Callable[[float], Awaitable[None]],
        rng: random.Random,
        client: object,
    ) -> Coroutine[object, object, object]: ...


fetch_with_retry = cast(
    _FetchCall,
    cost_map._fetch_remote_model_cost_map_with_retry,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
)


class _Reloaded(Protocol):
    model_cost_map: CostMap
    revision: str | None
    etag: str | None


class _Retryable(Protocol):
    retry_after_seconds: float | None


class _ScriptedRandom:
    def __init__(self, values: list[float]) -> None:
        self.values = values

    def uniform(self, lower: float, upper: float) -> float:
        del lower, upper
        return self.values.pop(0)


class _Sleeper:
    def __init__(self) -> None:
        self.seconds: list[float] = []

    async def sleep(self, seconds: float) -> None:
        self.seconds.append(seconds)


class _ScriptedClient:
    def __init__(self, replies: list[httpx.Response]) -> None:
        self.replies = replies
        self.calls: list[str] = []

    async def get(
        self,
        url: str,
        *,
        timeout: float | None = None,  # noqa: ASYNC109
    ) -> httpx.Response:
        del timeout
        self.calls.append(url)
        return self.replies.pop(0)


def _seconds(value: float) -> str:
    return format(Decimal(str(value)).normalize(), "f")


def _optional_seconds(value: float | None) -> str | None:
    return None if value is None else _seconds(value)


def _string(input_value: dict[str, Json], name: str) -> str:
    value = input_value.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    return None if value is None else _string(input_value, name)


def _int(input_value: dict[str, Json], name: str, default: int) -> int:
    value = input_value.get(name)
    if value is None:
        return default
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _headers(input_value: dict[str, Json]) -> Headers:
    value = input_value.get("headers", {})
    if not isinstance(value, dict):
        raise ValueError("headers must be an object")
    return {name: _string(value, name) for name in value}


def _cost_map(input_value: dict[str, Json]) -> CostMap:
    keys = input_value.get("model_keys", [])
    if not isinstance(keys, list):
        raise ValueError("model_keys must be a list")
    return {cast(str, key): {} for key in keys}


def _response(status: int, headers: Headers, body: str) -> httpx.Response:
    return httpx.Response(status_code=status, headers=headers, content=body.encode())


def _jitter(input_value: dict[str, Json]) -> _ScriptedRandom:
    values = input_value.get("jitter", [])
    if not isinstance(values, list):
        raise ValueError("jitter must be a list")
    return _ScriptedRandom([float(cast(str, value)) for value in values])


def _retryable(input_value: dict[str, Json]) -> object:
    header = _optional_string(input_value, "retry_after")
    header_values: Headers = {} if header is None else {"Retry-After": header}
    seconds = parse_retry_after(_response(200, header_values, ""))
    factory = cast(Callable[..., object], cost_map._FetchAttemptRetryable)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return factory(reason="synthetic", retry_after_seconds=seconds)


def _validate_map(input_value: dict[str, Json]) -> Json:
    fetched = _cost_map(input_value)
    backup = _int(input_value, "backup_model_count", 0)
    minimum = _int(input_value, "min_model_count", 50)
    ratio = float(_optional_string(input_value, "max_shrink_ratio") or "0.5")
    valid = GetModelCostMap.validate_model_cost_map(fetched, backup, minimum, ratio)  # pyright: ignore[reportUnknownMemberType]
    valid_dict = GetModelCostMap._check_is_valid_dict(fetched)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    count_ok = GetModelCostMap._check_model_count_not_reduced(fetched, backup, minimum, ratio)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return {
        "valid": valid,
        "is_valid_dict": valid_dict,
        "count_not_reduced": count_ok,
        "count": count_entries(fetched),
    }


def _classify(input_value: dict[str, Json]) -> Json:
    response = _response(
        _int(input_value, "status", 0), _headers(input_value), _string(input_value, "body")
    )
    outcome = classify_response(response, _string(input_value, "url"))
    if isinstance(outcome, ModelCostMapReloaded):
        reloaded = cast(_Reloaded, outcome)
        return cast(
            Json,
            {
                "kind": "reloaded",
                "revision": reloaded.revision,
                "etag": reloaded.etag,
                "models": sorted(reloaded.model_cost_map),
            },
        )
    if isinstance(outcome, ModelCostMapReloadUnavailable):
        return {"kind": "unavailable"}
    retryable = cast(_Retryable, outcome)
    return {
        "kind": "retryable",
        "retry_after_seconds": _optional_seconds(retryable.retry_after_seconds),
    }


def _parse_retry_after(input_value: dict[str, Json]) -> Json:
    response = _response(200, _headers(input_value), "")
    return {"retry_after_seconds": _optional_seconds(parse_retry_after(response))}


def _retry_wait(input_value: dict[str, Json]) -> Json:
    seconds = retry_wait(
        _retryable(input_value),
        _int(input_value, "attempt", 0),
        cast(random.Random, _jitter(input_value)),
    )
    return {"wait_seconds": _seconds(seconds)}


def _next_retry_wait(input_value: dict[str, Json]) -> Json:
    result = next_retry_wait(
        _retryable(input_value),
        _int(input_value, "attempt", 0),
        _int(input_value, "max_attempts", 3),
        cast(random.Random, _jitter(input_value)),
    )
    if isinstance(result, ModelCostMapReloadUnavailable):
        return {"kind": "unavailable"}
    return {"kind": "retry", "wait_seconds": _seconds(cast(float, result))}


def _fetch_with_retry(input_value: dict[str, Json]) -> Json:
    replies = input_value.get("replies", [])
    if not isinstance(replies, list):
        raise ValueError("replies must be a list")
    client = _ScriptedClient(
        [
            _response(
                _int(cast(dict[str, Json], reply), "status", 0),
                _headers(cast(dict[str, Json], reply)),
                _string(cast(dict[str, Json], reply), "body"),
            )
            for reply in replies
        ]
    )
    sleeper = _Sleeper()
    rng = _jitter(input_value)
    result = asyncio.run(
        fetch_with_retry(
            url=_string(input_value, "url"),
            timeout=_int(input_value, "timeout", 5),
            max_attempts=_int(input_value, "max_attempts", 3),
            sleep=sleeper.sleep,
            rng=cast(random.Random, rng),
            client=client,
        )
    )
    facts: dict[str, Json] = {
        "requests": len(client.calls),
        "sleeps": [_seconds(seconds) for seconds in sleeper.seconds],
    }
    if isinstance(result, ModelCostMapReloaded):
        reloaded = cast(_Reloaded, result)
        return cast(
            Json,
            {
                "kind": "reloaded",
                **facts,
                "revision": reloaded.revision,
                "etag": reloaded.etag,
                "models": sorted(reloaded.model_cost_map),
            },
        )
    return cast(Json, {"kind": "unavailable", **facts})


def _raw_blob_revision(input_value: dict[str, Json]) -> Json:
    return {"revision": git_blob_id(_string(input_value, "body").encode())}


Handler: TypeAlias = Callable[[dict[str, Json]], Json]
HANDLERS: dict[str, Handler] = {
    "raw_blob_revision": _raw_blob_revision,
    "validate_map": _validate_map,
    "classify_response": _classify,
    "parse_retry_after": _parse_retry_after,
    "retry_wait": _retry_wait,
    "next_retry_wait": _next_retry_wait,
    "fetch_with_retry": _fetch_with_retry,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = _string(input_value, "operation")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown catalog fetch operation: {operation}")
    return handler(input_value)
