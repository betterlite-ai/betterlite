import asyncio
import json
import random
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._model_info.catalog import catalog_state

DEFAULT_MAX_ATTEMPTS: Final = 3
DEFAULT_TIMEOUT_SECONDS: Final = 5


class _CostMapClient(Protocol):
    ...


_SyncSleep: TypeAlias = Callable[[float], None]
_AsyncSleep: TypeAlias = Callable[[float], Awaitable[None]]


class CostMapProvenance(TypedDict):
    source_revision: str | None
    etag: str | None


class CostMapSourceInfo(CostMapProvenance):
    source: str
    url: str | None
    is_env_forced: bool
    fallback_reason: str | None
    loaded_at: str | None


class _ReloadedPayload(TypedDict):
    status: Literal["reloaded"]
    model_cost_map: dict[str, JsonValue]
    revision: str | None
    etag: str | None


class _UnavailablePayload(TypedDict):
    status: Literal["unavailable"]
    reason: str


_ReloadPayload: TypeAlias = _ReloadedPayload | _UnavailablePayload


class _ReloadRequest(TypedDict):
    url: str
    timeout: float
    max_attempts: int


_RELOAD: Final[TypeAdapter[_ReloadPayload]] = TypeAdapter(_ReloadPayload)
_RELOADED: Final[TypeAdapter[_ReloadedPayload]] = TypeAdapter(_ReloadedPayload)
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
    sleep: _SyncSleep | _AsyncSleep | None,
    rng: random.Random | None,
    client: _CostMapClient | None,
    default_sleep: _SyncSleep | _AsyncSleep,
) -> None:
    if client is not None:
        raise UnsupportedCallArgument("client")
    if rng is not None:
        raise UnsupportedCallArgument("rng")
    if sleep is not None and sleep is not default_sleep:
        raise UnsupportedCallArgument("sleep")


def _reload_request(url: str, timeout: float, max_attempts: int) -> str:
    request: _ReloadRequest = {"url": url, "timeout": timeout, "max_attempts": max_attempts}
    return json.dumps(request)


def _reload_result(text: str) -> ModelCostMapReloadResult:
    payload = _RELOAD.validate_json(text)
    if payload["status"] == "unavailable":
        return ModelCostMapReloadUnavailable(reason=payload["reason"])
    return ModelCostMapReloaded(
        model_cost_map=payload["model_cost_map"],
        revision=payload["revision"],
        etag=payload["etag"],
    )


def get_model_cost_map(
    url: str,
    timeout: int = DEFAULT_TIMEOUT_SECONDS,
    max_attempts: int = DEFAULT_MAX_ATTEMPTS,
    sleep: _SyncSleep = time.sleep,
    rng: random.Random | None = None,
    client: _CostMapClient | None = None,
) -> dict[str, JsonValue]:
    _reject_injected_options(sleep, rng, client, time.sleep)
    text = catalog_state.native.reload_blocking(_reload_request(url, float(timeout), max_attempts))
    return _RELOADED.validate_json(text)["model_cost_map"]


async def refetch_model_cost_map(
    url: str,
    timeout: int = DEFAULT_TIMEOUT_SECONDS,
    max_attempts: int = DEFAULT_MAX_ATTEMPTS,
    sleep: _AsyncSleep = asyncio.sleep,
    rng: random.Random | None = None,
    client: _CostMapClient | None = None,
) -> ModelCostMapReloadResult:
    _reject_injected_options(sleep, rng, client, asyncio.sleep)
    request = _reload_request(url, float(timeout), max_attempts)
    text = await catalog_state.native.reload_async(request)
    return _reload_result(text)


def get_model_cost_map_provenance() -> CostMapProvenance:
    info = _SOURCE_INFO.validate_json(catalog_state.native.source_info())
    return CostMapProvenance(source_revision=info["source_revision"], etag=info["etag"])


# ast-grep-ignore: no-one-line-functions
def get_model_cost_map_source_info() -> CostMapSourceInfo:
    return _SOURCE_INFO.validate_json(catalog_state.native.source_info())


def get_model_cost_map_loaded_at() -> datetime | None:
    loaded_at = _SOURCE_INFO.validate_json(catalog_state.native.source_info())["loaded_at"]
    return None if loaded_at is None else datetime.fromisoformat(loaded_at)
