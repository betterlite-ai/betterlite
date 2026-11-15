from collections.abc import Callable
from typing import TypeVar

import litellm
from pydantic import JsonValue

from .capabilities import install
from .chat import credential, messages_value, options_value, supported_value
from .chunks import chunk_value
from .cost import cost_value
from .replies import reply_value
from .stt import stt_value
from .wire import Capabilities, Input, Object

litellm.suppress_debug_info = True

CallT = TypeVar("CallT")


def project(calls: list[CallT] | None, value: Callable[[CallT], JsonValue]) -> list[JsonValue]:
    resolved: list[CallT] = [] if calls is None else calls
    return [value(call) for call in resolved]


def run(input_value: Input) -> JsonValue:
    capabilities = input_value.get("capabilities")
    install(Capabilities() if capabilities is None else capabilities)
    result: Object = {
        "credentials": project(input_value.get("credentials"), credential),
        "supported": project(input_value.get("supported"), supported_value),
        "messages": project(input_value.get("messages"), messages_value),
        "options": project(input_value.get("options"), options_value),
        "replies": project(input_value.get("replies"), reply_value),
        "chunks": project(input_value.get("chunks"), chunk_value),
        "stt": project(input_value.get("stt"), stt_value),
        "costs": project(input_value.get("costs"), cost_value),
    }
    return result
