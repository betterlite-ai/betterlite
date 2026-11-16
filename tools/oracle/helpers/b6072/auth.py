import os

from pydantic import JsonValue

from .views import CHAT
from .wire import MANAGED_ENV, MODEL, HeaderCall, KeyCall


def activate(environment: dict[str, str]) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def header_value(call: HeaderCall) -> JsonValue:
    activate(call["environment"])
    return CHAT.validate_environment(
        call["headers"], MODEL, [], {}, call["litellm_params"], call["api_key"], None
    )


def key_value(call: KeyCall) -> JsonValue:
    activate(call["environment"])
    return CHAT.get_api_key(api_key=call["api_key"])
