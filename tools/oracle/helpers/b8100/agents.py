from typing import Literal, TypedDict

from litellm.llms.a2a.chat.transformation import A2AConfig
from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.types.agents import AgentResponse
from pydantic import JsonValue

from .support import Body


class AgentCase(TypedDict):
    name: str
    card: Body
    params: Body


class ProbeCase(TypedDict):
    agent_name: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None
    optional_params: Body


class RegistryInput(TypedDict):
    op: Literal["registry"]
    agents: list[AgentCase]
    probes: list[ProbeCase]


def register(agent: AgentCase) -> None:
    fields: Body = {
        "agent_id": agent["name"],
        "agent_name": agent["name"],
        "agent_card_params": agent["card"],
        "litellm_params": agent["params"],
    }
    found = AgentResponse.model_validate(fields)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/agents.py#L314
    global_agent_registry.register_agent(found)


def probe_view(probe: ProbeCase) -> JsonValue:
    optional_params: Body = dict(probe["optional_params"])
    resolved: tuple[str | None, str | None, Body | None] = (
        A2AConfig.resolve_agent_config_from_registry(
            probe["agent_name"],
            probe["api_base"],
            probe["api_key"],
            probe["headers"],
            optional_params,
        )
    )
    api_base, api_key, headers = resolved
    return {
        "api_base": api_base,
        "api_key": api_key,
        "headers": headers,
        "optional_params": optional_params,
    }


def registry(input_value: RegistryInput) -> JsonValue:
    for agent in input_value["agents"]:
        register(agent)
    return {"probes": [probe_view(probe) for probe in input_value["probes"]]}
