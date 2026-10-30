import asyncio
import types
from typing import TypeAlias, TypedDict, cast, final

from litellm.caching.redis_cluster_node_isolation import get_litellm_async_redis_cluster_class
from redis.cluster import get_node_name
from redis.commands import READ_COMMANDS
from redis.exceptions import (
    AskError,
    BusyLoadingError,
    ClusterDownError,
    ClusterError,
    MaxConnectionsError,
    MovedError,
    SlotNotCoveredError,
    TryAgainError,
)
from redis.exceptions import ConnectionError as RedisConnectionError
from redis.exceptions import TimeoutError as RedisTimeoutError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Outcome(TypedDict):
    kind: str
    node: str


class Input(TypedDict):
    version: str
    has_update_active_connections_for_reconnect: bool
    node: str
    ttl: int
    reinitialize_steps: int
    initialize: bool
    read_from_replicas: bool
    outcomes: list[Outcome]


@final
class _Node:
    def __init__(self, name: str, outcomes: list[Outcome], events: list[str]) -> None:
        self.name = name
        self._outcomes = outcomes
        self._events = events

    async def execute_command(self, *args: object, **_kwargs: object) -> object:  # noqa: C901  # maps fixture outcomes to real redis-py exceptions
        if args and args[0] == "ASKING":
            return "OK"
        if not self._outcomes:
            raise ClusterError("fixture command sequence exhausted")
        outcome = self._outcomes.pop(0)
        kind = outcome["kind"]
        if kind == "success":
            return "OK"
        if kind == "connection_error":
            raise RedisConnectionError
        if kind == "timeout":
            raise RedisTimeoutError
        if kind == "moved":
            raise MovedError(f"0 {self.name}:6379")
        if kind == "ask":
            raise AskError(f"0 {outcome['node']}:6379")
        if kind == "cluster_down":
            raise ClusterDownError("CLUSTERDOWN")
        if kind == "slot_not_covered":
            raise SlotNotCoveredError("slot not covered")
        if kind == "try_again":
            raise TryAgainError
        if kind == "busy_loading":
            raise BusyLoadingError
        if kind == "max_connections":
            raise MaxConnectionsError
        raise ClusterError(f"unknown fixture outcome: {kind}")

    async def disconnect(self) -> None:
        self._events.append(f"disconnected:{self.name}")

    def update_active_connections_for_reconnect(self) -> None:
        return None

    async def disconnect_free_connections(self) -> None:
        return None


@final
class _NodesManager:
    def __init__(self, nodes: dict[str, _Node], events: list[str]) -> None:
        self._nodes = nodes
        self._events = events
        self._moved_error: object = None

    @property
    def _moved_exception(self) -> object:
        return self._moved_error

    @_moved_exception.setter
    def _moved_exception(self, value: object) -> None:
        self._moved_error = value
        self._events.append("moved")

    def get_node_from_slot(self, _slot: int, _read_from_replicas: bool, _strategy: object) -> _Node:  # noqa: FBT001  # mirrors redis-py's positional protocol
        return next(iter(self._nodes.values()))

    def move_slot(self, _error: MovedError) -> None:
        self._events.append("moved")

    def move_node_to_end_of_cached_nodes(self, _name: str) -> None:
        return None


class _BaseCluster:
    def __init__(  # noqa: PLR0913  # mirrors the injected cluster fixture's explicit state
        self,
        *,
        node: _Node,
        nodes: dict[str, _Node],
        ttl: int,
        reinitialize_steps: int,
        initialize: bool,
        read_from_replicas: bool,
        events: list[str],
    ) -> None:
        self.RedisClusterRequestTTL = ttl
        self.reinitialize_counter = 0
        self.reinitialize_steps = reinitialize_steps
        self._initialize = initialize
        self.read_from_replicas = read_from_replicas
        self.load_balancing_strategy: object = None
        self.nodes_manager = _NodesManager(nodes, events)
        self._nodes = nodes
        self._node = node
        self._events = events

    def get_node(self, node_name: str) -> _Node:
        node = self._nodes.get(node_name.split(":", maxsplit=1)[0], self._node)
        self._events.append(f"asked:{node.name}")
        return node

    async def _determine_slot(self, *_args: object) -> int:
        return 0

    async def aclose(self) -> None:
        self._events.append("full_reinitialize")

    async def _execute_command(self, target_node: _Node, *args: object, **kwargs: object) -> object:  # noqa: C901  # mirrors upstream redirect branches
        asking = False
        moved = False
        redirect_addr: str | None = None
        ttl = self.RedisClusterRequestTTL
        while ttl > 0:
            ttl -= 1
            try:
                if asking:
                    target_node = self.get_node(node_name=redirect_addr or "")
                    await target_node.execute_command("ASKING")
                    asking = False
                elif moved:
                    slot = await self._determine_slot(*args)
                    target_node = self.nodes_manager.get_node_from_slot(
                        slot,
                        self.read_from_replicas and args[0] in READ_COMMANDS,
                        self.load_balancing_strategy if args[0] in READ_COMMANDS else None,
                    )
                    moved = False
                return await target_node.execute_command(*args, **kwargs)
            except (BusyLoadingError, MaxConnectionsError):
                raise
            except (RedisConnectionError, RedisTimeoutError):
                self._initialize = True
                raise
            except (ClusterDownError, SlotNotCoveredError):
                await self.aclose()
                await asyncio.sleep(0.25)
                raise
            except MovedError as error:
                self.reinitialize_counter += 1
                if (
                    self.reinitialize_steps
                    and self.reinitialize_counter % self.reinitialize_steps == 0
                ):
                    await self.aclose()
                    self.reinitialize_counter = 0
                else:
                    self.nodes_manager.move_slot(error)
                moved = True
            except AskError as error:
                redirect_addr = get_node_name(
                    host=str(getattr(error, "host", "")),
                    port=int(getattr(error, "port", 0)),
                )
                asking = True
            except TryAgainError:
                if ttl < self.RedisClusterRequestTTL / 2:
                    await asyncio.sleep(0.05)
        raise ClusterError("TTL exhausted.")


class _Fixture:
    def __init__(self, input_value: Input) -> None:
        self.input = input_value
        self.events: list[str] = []
        self.sleeps: list[float] = []
        outcomes = list(input_value["outcomes"])
        self.nodes = {input_value["node"]: _Node(input_value["node"], outcomes, self.events)}
        for outcome in outcomes:
            target = outcome["node"]
            if target:
                self.nodes.setdefault(target, _Node(target, outcomes, self.events))

    async def run(self) -> Json:
        node = self.nodes[self.input["node"]]
        node_class: type = (
            _Node
            if self.input["has_update_active_connections_for_reconnect"]
            else type("ClusterNode", (), {})
        )
        selected = get_litellm_async_redis_cluster_class(
            cluster_node_class=node_class,
            base_cluster_class=_BaseCluster,
        )
        cluster_type = cast(type[_BaseCluster], selected)
        if self.input["has_update_active_connections_for_reconnect"]:
            cluster = cluster_type(
                node=node,
                nodes=self.nodes,
                ttl=self.input["ttl"],
                reinitialize_steps=self.input["reinitialize_steps"],
                initialize=self.input["initialize"],
                read_from_replicas=self.input["read_from_replicas"],
                events=self.events,
            )
        else:
            cluster = object.__new__(cluster_type)
            _BaseCluster.__init__(
                cluster,
                node=node,
                nodes=self.nodes,
                ttl=self.input["ttl"],
                reinitialize_steps=self.input["reinitialize_steps"],
                initialize=self.input["initialize"],
                read_from_replicas=self.input["read_from_replicas"],
                events=self.events,
            )
            cluster.get_node = types.MethodType(_BaseCluster.get_node, cluster)
            cluster._determine_slot = (  # pyright: ignore[reportPrivateUsage]
                types.MethodType(  # install the fixture slot seam on the production class
                    _BaseCluster._determine_slot,  # noqa: SLF001  # bind the injected protected seam  # pyright: ignore[reportPrivateUsage]
                    cluster,
                )
            )
            cluster.aclose = types.MethodType(_BaseCluster.aclose, cluster)
        original_sleep = asyncio.sleep

        async def record_sleep(seconds: float) -> None:
            self.sleeps.append(seconds)
            self.events.append(f"slept:{seconds}")

        asyncio.sleep = record_sleep
        try:
            await cluster._execute_command(node, "GET", key="fixture")  # pyright: ignore[reportPrivateUsage]
            status = "success"
            asked_nodes = [
                event.removeprefix("asked:") for event in self.events if event.startswith("asked:")
            ]
            result_node: str | None = asked_nodes[-1] if asked_nodes else node.name
        except (
            RedisConnectionError,
            RedisTimeoutError,
            ClusterDownError,
            SlotNotCoveredError,
            TryAgainError,
            BusyLoadingError,
            MaxConnectionsError,
            ClusterError,
        ) as error:
            status = error_name(error)
            result_node = None
        finally:
            asyncio.sleep = original_sleep
        return cast(
            "Json",
            {
                "branch": "per_node_recovery"
                if self.input["has_update_active_connections_for_reconnect"]
                else "legacy",
                "status": status,
                "node": result_node,
                "events": self.events if status == "success" else [],
                "disconnected_nodes": [
                    event.removeprefix("disconnected:")
                    for event in self.events
                    if event.startswith("disconnected:")
                ],
                "reinitialize_counter": cluster.reinitialize_counter,
                "sleeps": self.sleeps,
                "read_from_replicas": self.input["read_from_replicas"],
                "version_warning": (
                    None
                    if self.input["has_update_active_connections_for_reconnect"]
                    or self.input["version"] == "5.3.1"
                    else (
                        f"redis-py {self.input['version']} is not a verified "
                        "cluster-isolation version"
                    )
                ),
            },
        )


def error_name(error: Exception) -> str:
    for error_type, name in (
        (RedisConnectionError, "connection"),
        (RedisTimeoutError, "timeout"),
        (ClusterDownError, "cluster_down"),
        (SlotNotCoveredError, "slot_not_covered"),
        (BusyLoadingError, "busy_loading"),
        (MaxConnectionsError, "max_connections"),
    ):
        if isinstance(error, error_type):
            return name
    return "ttl_exhausted"


def run(input_value: Input) -> Json:
    return asyncio.run(_Fixture(input_value).run())
