import json
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite import _native

AGENT_CARD_WELL_KNOWN_PATH: Final = "/.well-known/agent-card.json"
# ast-grep-ignore: no-long-names
PREV_AGENT_CARD_WELL_KNOWN_PATH: Final = "/.well-known/agent.json"
FOUNDRY_AGENT_CARD_PATH: Final = "/agentCard/v1.0"

JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def is_localhost_or_internal_url(url: str | None) -> bool:
    if url is None:
        return False
    return _native.is_localhost_or_internal_url(url)


# ast-grep-ignore: no-long-names
def normalize_agent_card_interfaces(agent_card: dict[str, JsonValue]) -> dict[str, JsonValue]:
    text = _native.normalize_agent_card_interfaces(json.dumps(agent_card))
    return JSON_OBJECT.validate_json(text)


# ast-grep-ignore: no-one-line-functions
def get_agent_card_url(agent_card: dict[str, JsonValue]) -> str | None:
    return _native.get_agent_card_url(json.dumps(agent_card))


# ast-grep-ignore: no-one-line-functions
def set_agent_card_url(agent_card: dict[str, JsonValue], url: str) -> dict[str, JsonValue]:
    return JSON_OBJECT.validate_json(_native.set_agent_card_url(json.dumps(agent_card), url))


# ast-grep-ignore: no-one-line-functions
def fix_agent_card_url(agent_card: dict[str, JsonValue], base_url: str) -> dict[str, JsonValue]:
    return JSON_OBJECT.validate_json(_native.fix_agent_card_url(json.dumps(agent_card), base_url))
