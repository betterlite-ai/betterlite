import asyncio
import json
from collections.abc import Callable
from pathlib import Path
from typing import TypeAlias, cast

from litellm.proxy.public_endpoints import public_endpoints as pe
from litellm.types.proxy.public_endpoints.public_endpoints import AgentCreateInfo

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _validated_agent_fields() -> list[dict[str, Json]]:
    raw_agents = asyncio.run(pe.get_agent_fields())
    validated = (AgentCreateInfo.model_validate(agent) for agent in raw_agents)
    return [cast(dict[str, Json], agent.model_dump()) for agent in validated]


def _find_agent(agents: list[dict[str, Json]], agent_type: str) -> dict[str, Json]:
    for agent in agents:
        if agent["agent_type"] == agent_type:
            return agent
    raise ValueError(f"no agent type {agent_type}")


def get_agent_fields(input_value: dict[str, Json]) -> Json:
    del input_value
    return cast(Json, _validated_agent_fields())


def get_agent_field(input_value: dict[str, Json]) -> Json:
    agent_type = input_value["agent_type"]
    if not isinstance(agent_type, str):
        raise ValueError("agent_type must be a string")
    return cast(Json, _find_agent(_validated_agent_fields(), agent_type))


def agent_credential_fields(input_value: dict[str, Json]) -> Json:
    agent_type = input_value["agent_type"]
    if not isinstance(agent_type, str):
        raise ValueError("agent_type must be a string")
    agent = _find_agent(_validated_agent_fields(), agent_type)
    return agent["credential_fields"]


def _normalize_credential_field(field: dict[str, Json]) -> dict[str, Json]:
    return {
        "key": field["key"],
        "label": field["label"],
        "placeholder": field.get("placeholder"),
        "tooltip": field.get("tooltip"),
        "required": field.get("required", False),
        "field_type": field.get("field_type", "text"),
        "options": field.get("options"),
        "default_value": field.get("default_value"),
        "include_in_litellm_params": field.get("include_in_litellm_params"),
        "validation_pattern": field.get("validation_pattern"),
        "validation_message": field.get("validation_message"),
    }


def _normalize_raw_agent(agent: dict[str, Json]) -> dict[str, Json]:
    credential_fields = agent.get("credential_fields", [])
    if not isinstance(credential_fields, list):
        raise ValueError("credential_fields must be a list")
    fields = (cast(dict[str, Json], field) for field in credential_fields)
    normalized_fields = cast(Json, [_normalize_credential_field(field) for field in fields])
    return {
        "agent_type": agent["agent_type"],
        "agent_type_display_name": agent["agent_type_display_name"],
        "description": agent.get("description"),
        "logo_url": agent.get("logo_url"),
        "inherit_credentials_from_provider": agent.get("inherit_credentials_from_provider"),
        "credential_fields": normalized_fields,
        "litellm_params_template": agent.get("litellm_params_template"),
        "model_template": agent.get("model_template"),
        "use_a2a_form_fields": agent.get("use_a2a_form_fields"),
    }


def agent_catalog_raw(input_value: dict[str, Json]) -> Json:
    del input_value
    base_path = Path(pe.__file__).parent.parent.parent / "proxy" / "public_endpoints"
    with (base_path / "agent_create_fields.json").open() as f:
        raw_agents = cast(list[dict[str, Json]], json.load(f))
    return [_normalize_raw_agent(agent) for agent in raw_agents]


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "get_agent_fields": get_agent_fields,
    "get_agent_field": get_agent_field,
    "agent_credential_fields": agent_credential_fields,
    "agent_catalog_raw": agent_catalog_raw,
}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function not in HANDLERS:
        raise ValueError(f"unknown b3191 function: {function}")
    return HANDLERS[function](input_value)
