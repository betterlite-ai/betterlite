import io
import os
from collections.abc import Callable
from contextlib import redirect_stdout
from typing import Final

import litellm
from pydantic import JsonValue

from .auth import sign_cases
from .base import base_cases
from .chat import chat_cases
from .failure import failure_cases
from .messages import message_cases
from .passthrough import passthrough_cases
from .reply import reply_cases

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"
# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

HANDLERS: Final[dict[str, Callable[[JsonValue], JsonValue]]] = {
    "standalone_mantle_base": base_cases,
    "standalone_mantle_auth": sign_cases,
    "chat_provider_prefix": chat_cases,
    "messages_auth_and_url": message_cases,
    "mantle_passthrough_path": passthrough_cases,
    "mantle_auth_failure": failure_cases,
    "mantle_passthrough_reply_shape": reply_cases,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("bedrock_mantle_settings takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    # LiteLLM logs below WARNING to stdout, which carries the helper's JSON answer
    with redirect_stdout(io.StringIO()):
        return handler(payload)
