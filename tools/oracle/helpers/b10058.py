from typing import TypedDict

from litellm.proxy.bug_report_config import safe_config_lines
from pydantic import JsonValue


class Input(TypedDict):
    config: dict[str, JsonValue]
    general_settings: dict[str, JsonValue]


def run(input_value: Input) -> list[str]:
    config = input_value["config"]
    general_settings = input_value["general_settings"]
    return list(safe_config_lines(config, general_settings))
