import asyncio
import json
import random
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import datetime
from typing import Final, Protocol, TypeAlias, TypedDict, Unpack

from pydantic import JsonValue, TypeAdapter

from betterlite import _native
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._model_info.catalog import _CATALOG

DEFAULT_MAX_ATTEMPTS: Final = 3
DEFAULT_TIMEOUT_SECONDS: Final = 5


class _CostMapClient(Protocol):
    ...


class CostMapOptions(TypedDict, total=False):
    sleep: Callable[[float], None] | Callable[[float], Awaitable[None]]
    rng: random.Random
    client: _CostMapClient


class RefetchOptions(CostMapOptions, total=False):
    timeout: int
    max_attempts: int


class CostMapProvenance(TypedDict):
    source_revision: str | None
    etag: str | None


class CostMapSourceInfo(CostMapProvenance):
    source: str
    url: str | None
    is_env_forced: bool
    fallback_reason: str | None
    loaded_at: str | None


class _StatusPayload(TypedDict):
    status: str


class _ReloadedPayload(TypedDict):
    model_cost_map: dict[str, JsonValue]
    revision: str | None
    etag: str | None


class _UnavailablePayload(TypedDict):
    reason: str


class _ReloadRequest(TypedDict):
    url: str
    timeout: float
    max_attempts: int


_STATUS: Final[TypeAdapter[_StatusPayload]] = TypeAdapter(_StatusPayload)
_RELOADED: Final[TypeAdapter[_ReloadedPayload]] = TypeAdapter(_ReloadedPayload)
_UNAVAILABLE: Final[TypeAdapter[_UnavailablePayload]] = TypeAdapter(_UnavailablePayload)
_SOURCE_INFO: Final[TypeAdapter[CostMapSourceInfo]] = TypeAdapter(CostMapSourceInfo)


@dataclass(frozen=True, slots=True)
class ModelCostMapReloaded:
    model_cost_map: dict[str, JsonValue]
    revision: str | None = None
    etag: str | None = None


@dataclass(frozen=True, slots=True)
class ModelCostMapReloadUnavailable:
    reason: str


ModelCostMapReloadResult: TypeAlias = ModelCostMapReloaded | ModelCostMapReloadUnavailable


def _reject_injected_options(
    options: CostMapOptions,
    default_sleep: Callable[[float], None] | Callable[[float], Awaitable[None]],
) -> None:
    client = options.get("client")
    if client is not None:
        raise UnsupportedCallArgument("client")
    rng = options.get("rng")
    if rng is not None:
        raise UnsupportedCallArgument("rng")
    sleep = options.get("sleep")
    if sleep is not None and sleep is not default_sleep:
        raise UnsupportedCallArgument("sleep")


def _reload_request(url: str, timeout: float, max_attempts: int) -> str:
    request: _ReloadRequest = {
        "url": url,
        "timeout": timeout,
        "max_attempts": max_attempts,
    }
    return json.dumps(request)


def _reload_result(text: str) -> ModelCostMapReloadResult:
    status = _STATUS.validate_json(text)["status"]
    match status:
        case "reloaded":
            loaded = _RELOADED.validate_json(text)
            return ModelCostMapReloaded(
                model_cost_map=loaded["model_cost_map"],
                revision=loaded["revision"],
                etag=loaded["etag"],
            )
        case "unavailable":
            return ModelCostMapReloadUnavailable(reason=_UNAVAILABLE.validate_json(text)["reason"])
    raise ValueError(f"the native reload returned an unknown status {status}")


def get_model_cost_map(
    url: str,
    timeout: int = DEFAULT_TIMEOUT_SECONDS,
    max_attempts: int = DEFAULT_MAX_ATTEMPTS,
    **options: Unpack[CostMapOptions],
) -> dict[str, JsonValue]:
    _reject_injected_options(options, time.sleep)
    text = _CATALOG.native.reload_blocking(_reload_request(url, float(timeout), max_attempts))
    return _RELOADED.validate_json(text)["model_cost_map"]


async def refetch_model_cost_map(
    url: str,
    **options: Unpack[RefetchOptions],
) -> ModelCostMapReloadResult:
    _reject_injected_options(options, asyncio.sleep)
    timeout = options.get("timeout")
    max_attempts = options.get("max_attempts")
    limit = DEFAULT_TIMEOUT_SECONDS if timeout is None else timeout
    attempts = DEFAULT_MAX_ATTEMPTS if max_attempts is None else max_attempts
    text = await _CATALOG.native.reload_async(_reload_request(url, float(limit), attempts))
    return _reload_result(text)


def get_model_cost_map_provenance() -> CostMapProvenance:
    info = _SOURCE_INFO.validate_json(_CATALOG.native.source_info())
    return CostMapProvenance(source_revision=info["source_revision"], etag=info["etag"])


def get_model_cost_map_source_info() -> CostMapSourceInfo:
    return _SOURCE_INFO.validate_json(_CATALOG.native.source_info())


def get_model_cost_map_loaded_at() -> datetime | None:
    loaded_at = _SOURCE_INFO.validate_json(_CATALOG.native.source_info())["loaded_at"]
    if loaded_at is None:
        return None
    return datetime.fromisoformat(loaded_at)
