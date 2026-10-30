from typing import Final, TypeAlias, TypedDict, cast, final

from litellm.caching.redis_cluster_node_isolation import (
    get_litellm_async_redis_cluster_class,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CONSECUTIVE_TIMEOUTS_BEFORE_REINIT = 3


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
class _LegacyNode:
    name: Final = "node"


@final
class _NewerNode:
    name: Final = "node"

    def update_active_connections_for_reconnect(self) -> None:
        pass


class _BaseCluster:
    def __init__(self, **_kwargs: object) -> None:
        pass


def selected_branch(input_value: Input) -> str:
    node_class: type = (
        _NewerNode if input_value["has_update_active_connections_for_reconnect"] else _LegacyNode
    )
    selected = get_litellm_async_redis_cluster_class(
        cluster_node_class=node_class,
        base_cluster_class=_BaseCluster,
    )
    if selected.__name__.endswith("TimeoutTolerant"):
        return "per_node_recovery"
    return "legacy"


def event_name(kind: str, node: str) -> str:
    if kind in {"moved", "full_reinitialize", "slept"}:
        return kind
    return f"{kind}:{node}"


@final
class Simulation:
    def __init__(self, input_value: Input) -> None:
        self.input = input_value
        self.branch = selected_branch(input_value)
        self.current_node = input_value["node"]
        self.timeout_counts: dict[str, int] = {}
        self.disconnected: list[str] = []
        self.events: list[str] = []
        self.sleeps: list[float] = []
        self.reinitialize_counter = 0
        self.ttl = input_value["ttl"]
        self.outcome_index = 0
        self.asking_node: str | None = None
        self.moved = False
        self.status = "ttl_exhausted"

    def run(self) -> Json:
        while self.ttl > 0 and self.outcome_index < len(self.input["outcomes"]):
            self.ttl -= 1
            self.prepare_node()
            outcome = self.input["outcomes"][self.outcome_index]
            self.outcome_index += 1
            if self.process_outcome(outcome):
                break
        return self.output()

    def prepare_node(self) -> None:
        if self.asking_node is not None:
            self.current_node = self.asking_node
            self.events.append(event_name("asked", self.current_node))
            self.asking_node = None
        elif self.moved:
            self.current_node = self.input["node"]
            self.moved = False

    def process_outcome(self, outcome: Outcome) -> bool:
        kind = outcome["kind"]
        done = False
        if kind == "success":
            _ = self.timeout_counts.pop(self.current_node, None)
            self.status = "success"
            done = True
        elif kind == "timeout":
            self.handle_timeout()
            done = True
        elif kind == "connection_error":
            self.disconnected.append(self.current_node)
            self.events.append(event_name("disconnected", self.current_node))
            self.status = "connection"
            done = True
        elif kind in {"moved", "ask"}:
            self.handle_redirect(kind, outcome)
        elif kind in {"cluster_down", "slot_not_covered"}:
            self.handle_topology_error(kind)
            done = True
        elif kind == "try_again":
            self.handle_try_again()
        else:
            self.status = kind
            done = True
        return done

    def handle_timeout(self) -> None:
        count = self.timeout_counts.get(self.current_node, 0) + 1
        if self.branch == "legacy":
            self.disconnected.append(self.current_node)
            self.events.append(event_name("disconnected", self.current_node))
        elif count >= CONSECUTIVE_TIMEOUTS_BEFORE_REINIT:
            _ = self.timeout_counts.pop(self.current_node, None)
        else:
            self.timeout_counts[self.current_node] = count
        self.status = "timeout"

    def handle_redirect(self, kind: str, outcome: Outcome) -> None:
        if kind == "moved":
            self.reinitialize_counter += 1
            steps = self.input["reinitialize_steps"]
            if steps and self.reinitialize_counter % steps == 0:
                self.reinitialize_counter = 0
                self.events.append(event_name("full_reinitialize", self.current_node))
            self.events.append(event_name("moved", self.current_node))
            self.moved = True
        else:
            self.asking_node = outcome["node"]

    def handle_topology_error(self, kind: str) -> None:
        self.events.append(event_name("full_reinitialize", self.current_node))
        self.sleeps.append(0.25)
        self.events.append(event_name("slept", self.current_node))
        self.status = "cluster_down" if kind == "cluster_down" else "slot_not_covered"

    def handle_try_again(self) -> None:
        if self.ttl < self.input["ttl"] / 2:
            self.sleeps.append(0.05)
            self.events.append(event_name("slept", self.current_node))

    def output(self) -> Json:
        return cast("Json", {
            "branch": self.branch,
            "status": self.status,
            "node": self.current_node if self.status == "success" else None,
            "events": self.events if self.status == "success" else [],
            "disconnected_nodes": self.disconnected,
            "reinitialize_counter": self.reinitialize_counter,
            "sleeps": self.sleeps,
            "read_from_replicas": self.input["read_from_replicas"],
            "version_warning": (
                None
                if self.input["version"] == "5.3.1"
                else f"redis-py {self.input['version']} is not a verified cluster-isolation version"
            ),
        })


def run(input_value: Input) -> Json:
    return Simulation(input_value).run()
