import json
from collections.abc import Sequence

from pydantic import JsonValue

import betterlite
from betterlite import _native
from betterlite._tokens.wire import message_json
from betterlite.types.utils import Message


# LiteLLM returns the caller's limit when the model or count fails, and the native side does too
def get_modified_max_tokens(
    model: str,
    base_model: str,
    messages: Sequence[dict[str, JsonValue] | Message] | None,
    user_max_tokens: int | None,
    buffer_perc: float | None,
    buffer_num: float | None,
) -> int | None:
    request: dict[str, JsonValue] = {
        "model": model,
        "base_model": base_model,
        "user_max_tokens": user_max_tokens,
        "buffer_perc": buffer_perc,
        "buffer_num": buffer_num,
        "disable_token_counter": betterlite.disable_token_counter is True,
    }
    if messages is not None:
        request["messages"] = [message_json(message) for message in messages]
    return _native.modified_max_tokens(json.dumps(request))
