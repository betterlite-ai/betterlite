from typing import TypedDict

from litellm.proxy import proxy_server
from pydantic import JsonValue


class Input(TypedDict):
    general_settings: dict[str, JsonValue]
    object_type: str


def run(input_value: Input) -> bool:
    # dispatch.py runs one call per process, so the global starts empty and goes back to it
    proxy_server.general_settings = input_value["general_settings"]
    try:
        return proxy_server.should_load_db_object(object_type=input_value["object_type"])
    finally:
        proxy_server.general_settings = {}
