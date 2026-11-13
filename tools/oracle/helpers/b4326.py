import asyncio
from collections.abc import Coroutine
from typing import Protocol, TypeAlias, cast
from unittest import mock

from litellm._service_logger import ServiceLogging  # pyright: ignore[reportPrivateUsage]
from litellm.caching.redis_cache import (
    RedisCache,
    _AsyncRedisCommands,  # pyright: ignore[reportPrivateUsage]
)
from litellm.caching.redis_cluster_cache import RedisClusterCache
from redis._parsers.helpers import (
    parse_client_list,  # pyright: ignore[reportUnknownVariableType]
    parse_info,  # pyright: ignore[reportUnknownVariableType]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _ConnectionTest(Protocol):
    def test_connection(self) -> Coroutine[object, object, dict[str, Json]]: ...


class _ScriptedPing:
    def __init__(self, *, answer: bool | str, error_text: str) -> None:
        self.answer = answer
        self.error_text = error_text

    async def ping(self) -> bool:
        if self.answer == "error":
            raise RuntimeError(self.error_text)
        return bool(self.answer)

    async def aclose(self) -> None:
        pass


class _RecordingLogger:
    def __init__(self) -> None:
        self.events: list[Json] = []

    async def async_service_success_hook(
        self,
        service: object,  # noqa: ARG002 - the litellm keyword name
        call_type: str,
        duration: float,  # noqa: ARG002 - the litellm keyword name
        caller: object = None,  # noqa: ARG002 - the litellm keyword name
    ) -> None:
        self.events.append({"call_type": call_type, "outcome": "success", "error": None})

    async def async_service_failure_hook(
        self,
        service: object,  # noqa: ARG002 - the litellm keyword name
        duration: float,  # noqa: ARG002 - the litellm keyword name
        error: str | Exception,
        call_type: str,
        caller: object = None,  # noqa: ARG002 - the litellm keyword name
    ) -> None:
        self.events.append(
            {"call_type": call_type, "outcome": "failure", "error": str(error)}
        )


class _ScriptedCommands:
    def __init__(self, outcome: str, error_text: str) -> None:
        self.outcome = outcome
        self.error_text = error_text

    async def ping(self) -> bool:
        if self.outcome == "error":
            raise RuntimeError(self.error_text)
        return True


class _ProbeCache(RedisCache):
    def __init__(  # pyright: ignore[reportMissingSuperCall]
        self, logger: _RecordingLogger, commands: _ScriptedCommands
    ) -> None:
        self.logger = logger
        self.commands = commands
        self.service_logger_obj = cast(ServiceLogging, logger)

    def _async_commands(self) -> _AsyncRedisCommands:
        return cast(_AsyncRedisCommands, self.commands)


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str:
    if value is None:
        return ""
    return string_value(value, name)


def ping_value(value: Json) -> bool | str:
    if isinstance(value, bool):
        return value
    if value == "error":
        return "error"
    raise ValueError("ping must be true, false or 'error'")


def _connection_test(input_value: dict[str, Json]) -> Json:
    family = string_value(input_value.get("family"), "family")
    ping = ping_value(input_value.get("ping"))
    error_text = optional_string(input_value.get("error_text"), "error_text")
    client = _ScriptedPing(answer=ping, error_text=error_text)
    cache: RedisCache
    if family == "cluster":
        cache = object.__new__(RedisClusterCache)
    elif family == "standalone":
        cache = object.__new__(RedisCache)
    else:
        raise ValueError(f"unsupported family {family}")
    cache.redis_kwargs = {}
    with mock.patch("litellm._redis.get_redis_async_client", return_value=client):
        return asyncio.run(cast(_ConnectionTest, cache).test_connection())


async def _run_ping(cache: _ProbeCache) -> Json:
    result: bool | None
    error: str | None
    try:
        result = await cache.ping()
        error = None
    except Exception as exception:  # noqa: BLE001 - the scripted failure is the case
        result = None
        error = str(exception)
    await asyncio.sleep(0)
    return {"result": result, "error": error, "events": cache.logger.events}


def _ping_events(input_value: dict[str, Json]) -> Json:
    outcome = string_value(input_value.get("outcome"), "outcome")
    if outcome not in ("success", "error"):
        raise ValueError("outcome must be success or error")
    error_text = optional_string(input_value.get("error_text"), "error_text")
    cache = _ProbeCache(_RecordingLogger(), _ScriptedCommands(outcome, error_text))
    return asyncio.run(_run_ping(cache))


def _parse_text(input_value: dict[str, Json]) -> str:
    return string_value(input_value.get("text"), "text")


def _parse_client_list_output(text: str) -> Json:
    try:
        parsed = cast(object, parse_client_list(text))
    except KeyError:
        return {"error": "orphan_token"}
    return cast(Json, parsed)


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    if function == "parse_info":
        parsed = cast(object, parse_info(_parse_text(input_value)))
        return cast(Json, parsed)
    if function == "parse_client_list":
        return _parse_client_list_output(_parse_text(input_value))
    if function == "connection_test":
        return _connection_test(input_value)
    if function == "ping_events":
        return _ping_events(input_value)
    raise ValueError(f"unsupported function {function}")
