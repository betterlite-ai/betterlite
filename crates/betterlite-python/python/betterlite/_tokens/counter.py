import asyncio
import json
import weakref
from collections.abc import Awaitable, Callable, Sequence
from typing import Final, Generic, ParamSpec, TypeVar

from pydantic import JsonValue

import betterlite
from betterlite import _native
from betterlite._tokens.codec import CustomTokenizer, tokenizer_handle
from betterlite._tokens.wire import JSON, message_json
from betterlite.types.utils import Message

# litellm's TOKEN_COUNTER_MAX_CONCURRENT_COUNTS default, the width of the offload semaphore
MAX_CONCURRENT_COUNTS: Final = 4

P = ParamSpec("P")
R = TypeVar("R")

# litellm keeps one limiter per event loop, so a second loop does not wait on the first
_LIMITERS: Final[weakref.WeakKeyDictionary[asyncio.AbstractEventLoop, asyncio.Semaphore]] = (
    weakref.WeakKeyDictionary()
)


def text_to_count(
    text: str | list[str],
    tools: list[dict[str, JsonValue]] | None,
    tool_choice: dict[str, JsonValue] | str | None,
) -> str:
    if bool(tools) or tool_choice is not None:
        raise ValueError("tools or tool_choice cannot be set if using text")
    match text:
        case str():
            return text
        case list():
            return "".join(text)


def token_counter(
    model: str = "",
    custom_tokenizer: CustomTokenizer | None = None,
    text: str | list[str] | None = None,
    messages: Sequence[dict[str, JsonValue] | Message] | None = None,
    count_response_tokens: bool | None = False,
    tools: list[dict[str, JsonValue]] | None = None,
    tool_choice: dict[str, JsonValue] | str | None = None,
    use_default_image_token_count: bool | None = False,
    default_token_count: int | None = None,
) -> int:
    if betterlite.disable_token_counter is True:
        return 0
    if text is not None and messages is not None:
        raise ValueError("text and messages cannot both be set")
    request: dict[str, JsonValue] = {"model": model}
    if custom_tokenizer is not None:
        request["custom_tokenizer"] = tokenizer_handle(custom_tokenizer)
    if text is not None:
        request["text"] = text_to_count(text, tools, tool_choice)
    elif messages is not None:
        request["messages"] = [message_json(message) for message in messages]
        request["count_response_tokens"] = count_response_tokens
        if tools is not None:
            request["tools"] = JSON.validate_python(tools)
        if tool_choice is not None:
            request["tool_choice"] = JSON.validate_python(tool_choice)
    else:
        raise ValueError("Either text or messages must be provided")
    return _native.token_count(json.dumps(request))


async def run_offloaded(function: Callable[P, R], *args: P.args, **kwargs: P.kwargs) -> R:
    loop = asyncio.get_running_loop()
    limiter = _LIMITERS.get(loop)
    if limiter is None:
        limiter = asyncio.Semaphore(MAX_CONCURRENT_COUNTS)
        _LIMITERS[loop] = limiter
    async with limiter:
        return await asyncio.to_thread(function, *args, **kwargs)


class _Offloaded(Generic[P, R]):
    __slots__ = ("function",)

    def __init__(self, function: Callable[P, R]) -> None:
        self.function = function

    async def __call__(self, *args: P.args, **kwargs: P.kwargs) -> R:
        return await run_offloaded(self.function, *args, **kwargs)


# ast-grep-ignore: no-one-line-functions
def offload_token_count(function: Callable[P, R]) -> Callable[P, Awaitable[R]]:
    return _Offloaded(function)
