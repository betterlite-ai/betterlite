from collections.abc import Callable, Mapping, Sequence
from typing import Protocol, TypedDict

import litellm
from litellm.proxy.agent_endpoints import agent_registry
from litellm.types.agents import AgentResponse
from pydantic import JsonValue, TypeAdapter


class Registry(Protocol):
    def load_agents_from_config(
        self, agent_config: Sequence[Mapping[str, JsonValue]] | None = None
    ) -> None: ...

    def load_agents_from_db_and_config(
        self,
        agent_config: Sequence[Mapping[str, JsonValue]] | None = None,
        db_agents: Sequence[Mapping[str, JsonValue]] | None = None,
    ) -> Sequence[AgentResponse]: ...

    def get_agent_list(
        self, agent_names: Sequence[str] | None = None
    ) -> tuple[AgentResponse, ...]: ...

    def get_public_agent_list(self) -> tuple[AgentResponse, ...]: ...

    def ids_for_agent(self, agent_id: str) -> frozenset[str]: ...

    def stable_agent_id(self, agent_id: str) -> str: ...

    def get_agent_by_id(self, agent_id: str) -> AgentResponse | None: ...

    def get_agent_by_name(self, agent_name: str) -> AgentResponse | None: ...

    def deregister_agent(self, agent_name: str) -> None: ...


class Snapshot(TypedDict):
    agent_id: str
    agent_name: str


class Output(TypedDict):
    agents: list[Snapshot]
    ids_for: dict[str, list[str]]
    stable_ids: list[str]
    by_id: dict[str, Snapshot | None]
    by_name: dict[str, Snapshot | None]
    public: list[Snapshot]
    after_deregister: list[Snapshot]


class Input(TypedDict):
    config: list[dict[str, JsonValue]] | None
    reload: bool
    reload_config: list[dict[str, JsonValue]] | None
    db_agents: list[dict[str, JsonValue]]
    public_groups: list[str] | None
    ids_for: list[str]
    stable_ids: list[str]
    by_id: list[str]
    by_name: list[str]
    deregister: list[str]


REGISTRY: Callable[[], Registry] = TypeAdapter(
    Callable[[], Registry]
).validate_python(agent_registry.AgentRegistry)
SNAPSHOT: TypeAdapter[Snapshot] = TypeAdapter(Snapshot)
INPUT: TypeAdapter[Input] = TypeAdapter(Input)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


def snapshots(agents: Sequence[AgentResponse]) -> list[Snapshot]:
    return [
        SNAPSHOT.validate_json(agent.model_dump_json()) for agent in agents
    ]


def lookup(agent: AgentResponse | None) -> Snapshot | None:
    if agent is None:
        return None
    return SNAPSHOT.validate_json(agent.model_dump_json())


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    registry = REGISTRY()
    registry.load_agents_from_config(parsed["config"])
    if parsed["reload"]:
        registry.load_agents_from_db_and_config(parsed["reload_config"], parsed["db_agents"])
    agents = snapshots(registry.get_agent_list())
    ids_for = {agent_id: sorted(registry.ids_for_agent(agent_id)) for agent_id in parsed["ids_for"]}
    stable_ids = [registry.stable_agent_id(agent_id) for agent_id in parsed["stable_ids"]]
    by_id = {agent_id: lookup(registry.get_agent_by_id(agent_id)) for agent_id in parsed["by_id"]}
    by_name = {
        agent_name: lookup(registry.get_agent_by_name(agent_name))
        for agent_name in parsed["by_name"]
    }
    litellm.public_agent_groups = parsed["public_groups"]
    public = snapshots(registry.get_public_agent_list())
    for agent_name in parsed["deregister"]:
        registry.deregister_agent(agent_name)
    after_deregister = snapshots(registry.get_agent_list())
    result: Output = {
        "agents": agents,
        "ids_for": ids_for,
        "stable_ids": stable_ids,
        "by_id": by_id,
        "by_name": by_name,
        "public": public,
        "after_deregister": after_deregister,
    }
    return JSON.validate_python(result)
