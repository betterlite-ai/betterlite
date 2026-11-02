from typing import TypeAlias

from litellm.llms.codex.harness.transformation import config_overrides
from litellm.llms.opencode.harness.transformation import validate_user_config

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(argument: Json) -> dict[str, Json]:
    if not isinstance(argument, dict):
        raise ValueError("b5081 input must be an object")
    harness = argument.get("harness")
    config = argument.get("config")
    if not isinstance(config, dict):
        raise ValueError("b5081 input must contain a config object")
    if harness == "codex":
        config_overrides(config)
    elif harness == "opencode":
        validate_user_config(config)
    else:
        raise ValueError("b5081 harness must be codex or opencode")
    return {"valid": True}
