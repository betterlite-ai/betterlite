import asyncio
from typing import Literal

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.routing_prisma_wrapper import RoutingPrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue as Json
from typing_extensions import override

from .contracts import Scenario
from .fakes import FakeWrapper, ScriptedLock


def wrapper_for(
    target: Literal["writer", "reader"],
    writer: FakeWrapper,
    reader: FakeWrapper,
    *,
    routing: bool,
) -> FakeWrapper:
    if target == "writer" or not routing:
        return writer
    return reader


def failed_repairs(
    entry: Scenario,
    writer: FakeWrapper,
    reader: FakeWrapper,
    *,
    routing: bool,
) -> dict[PrismaWrapper, int]:
    repairs: dict[PrismaWrapper, int] = {}
    for repair in entry["failed"]:
        target = wrapper_for(repair["target"], writer, reader, routing=routing)
        repairs[target] = repair["generation"]
    return repairs


class FakeClient(PrismaClient):
    def __init__(
        self,
        entry: Scenario,
        db: PrismaWrapper | RoutingPrismaWrapper,
        writer: FakeWrapper,
        reader: FakeWrapper,
        effects: list[Json],
    ) -> None:
        settings = entry["settings"]
        self.db = db
        self.effects = effects
        self._db_reconnect_lock = ScriptedLock(entry["lock"], effects)
        self._db_last_reconnect_attempt_ts = entry["last_attempt"]
        self._db_reconnect_cooldown_seconds = settings["cooldown_seconds"]
        self._db_watchdog_reconnect_timeout_seconds = settings["watchdog_timeout_ms"] / 1000
        self._db_auth_reconnect_timeout_seconds = settings["auth_timeout_ms"] / 1000
        self._db_auth_reconnect_lock_timeout_seconds = settings["auth_lock_timeout_ms"] / 1000
        self._consecutive_reconnect_failures = entry["failures"]
        self._failed_recreate_generations = failed_repairs(
            entry, writer, reader, routing=entry["routing"]
        )
        self._reconnect_escalation_threshold = settings["escalation_threshold"]
        self._engine_confirmed_dead = entry["dead"]
        self._engine_pid = 0 if entry["engine_state"] == "unknown" else 1234
        self._engine_state = entry["engine_state"]
        self._probe = entry["writer_read_only"]
        self._probe_target = writer
        self._bump_on_probe = entry["bump_on_probe"]

    @override
    def _is_engine_alive(self) -> bool:
        return self._engine_state != "dead"

    @override
    def _cleanup_engine_watcher(self) -> None:
        self.effects.append({"effect": "cleanup"})

    @override
    async def _start_engine_watcher(self) -> None:
        self.effects.append({"effect": "rearm"})

    @override
    async def _writer_is_read_only(self, writer: PrismaWrapper) -> bool:
        self.effects.append({"effect": "probe"})
        if self._bump_on_probe:
            self._probe_target.bump_generation()
        if self._probe["pending"]:
            await asyncio.sleep(3600)
        if self._probe["error"] is not None:
            raise RuntimeError(self._probe["error"])
        return self._probe["value"] is True

    def failure_count(self) -> int:
        return self._consecutive_reconnect_failures

    def engine_dead(self) -> bool:
        return self._engine_confirmed_dead

    def failed_for(self, wrapper: PrismaWrapper) -> bool:
        if wrapper not in self._failed_recreate_generations:
            return False
        repair = self._failed_recreate_generations[wrapper]
        return repair == wrapper.engine_generation
