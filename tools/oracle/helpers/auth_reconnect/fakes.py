import asyncio
from typing import Literal

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from pydantic import JsonValue as Json
from typing_extensions import override

from .contracts import (
    LockInput,
    LogArgument,
    OpaqueHttpClient,
    Outcome,
    logged_fields,
)


class Recorder:
    def __init__(self, effects: list[Json]) -> None:
        self.effects = effects

    def debug(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "debug", "fields": fields})

    def info(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "info", "fields": fields})

    def warning(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "warn", "fields": fields})

    def error(self, message: str, *args: LogArgument) -> None:
        fields = logged_fields(message, args)
        self.effects.append({"effect": "log", "level": "error", "fields": fields})


class ScriptedTime:
    def __init__(self, values: list[float]) -> None:
        self.values = values
        self.reads = 0

    def time(self) -> float:
        index = min(self.reads, len(self.values) - 1)
        self.reads += 1
        return self.values[index]


class FakeWrapper(PrismaWrapper):
    def __init__(
        self,
        generation: int,
        recreation: Outcome,
        selection: Outcome,
        effects: list[Json],
    ) -> None:
        self._engine_generation = generation
        self.recreation = recreation
        self.selection = selection
        self.effects = effects

    @override
    async def recreate_prisma_client(
        self,
        new_db_url: str,
        http_client: OpaqueHttpClient | None = None,
        *,
        expected_generation: int | None = None,
    ) -> bool:
        self.effects.append(
            {"effect": "recreate", "expected_generation": expected_generation}
        )
        if self.recreation["pending"]:
            await asyncio.sleep(3600)
        if self.recreation["error"] is not None:
            raise RuntimeError(self.recreation["error"])
        return self.recreation["value"] is True

    async def query_raw(self, _query: str) -> list[dict[str, Json]]:
        self.effects.append({"effect": "select_one"})
        if self.selection["pending"]:
            await asyncio.sleep(3600)
        if self.selection["error"] is not None:
            raise RuntimeError(self.selection["error"])
        return []

    def bump_generation(self) -> None:
        self._engine_generation += 1


class Stale:
    """Stand-in for LiteLLM's private _StaleReadEngine: the observed handle and its generation.

    https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L4351
    """

    def __init__(self, wrapper: PrismaWrapper, generation: int) -> None:
        self.wrapper = wrapper
        self.generation = generation

    def is_still_live(self, current: PrismaWrapper) -> bool:
        return self.wrapper is current and self.generation == current.engine_generation


class FakeRouting(RoutingPrismaWrapper):
    def __init__(
        self,
        writer: FakeWrapper,
        reader: FakeWrapper,
        *,
        reader_unavailable: bool,
    ) -> None:
        super().__init__(writer, reader)
        self.writer_fake = writer
        self._reader_unavailable = reader_unavailable

    @override
    def mark_writer_recovered(self) -> None:
        self.writer_fake.effects.append({"effect": "mark_writer_recovered"})
        super().mark_writer_recovered()

    @override
    async def recreate_prisma_client(
        self,
        new_db_url: str,
        http_client: OpaqueHttpClient | None = None,
        *,
        expected_generation: int | None = None,
    ) -> bool:
        return await self.writer_fake.recreate_prisma_client(
            new_db_url,
            http_client=http_client,
            expected_generation=expected_generation,
        )


class ScriptedLock(asyncio.Lock):
    def __init__(self, script: LockInput, effects: list[Json]) -> None:
        super().__init__()
        self.mode = script["mode"]
        self.error = script["error"]
        self.effects = effects

    @override
    async def acquire(self) -> Literal[True]:
        self.effects.append({"effect": "lock_acquire"})
        if self.mode == "error":
            message = self.error
            if message is None:
                message = "the reconnect lock is unavailable"
            raise RuntimeError(message)
        if self.mode == "busy":
            await asyncio.sleep(3600)
        return await super().acquire()

    @override
    def release(self) -> None:
        self.effects.append({"effect": "lock_release"})
        super().release()
