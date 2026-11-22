import asyncio
import time
from collections.abc import Awaitable
from typing import Protocol

from pydantic import JsonValue

from betterlite._completion.arguments import CallValue
from betterlite._completion.completion import (
    call_arguments_from,
    completion_from_arguments,
    settle_outcome,
)
from betterlite._completion.stream import CompletionOutcome


class RetryCall(Protocol):
    def __call__(
        self, *args: JsonValue, **kwargs: CallValue
    ) -> CompletionOutcome | Awaitable[CompletionOutcome]: ...


def attempts_for(num_retries: int | None) -> int:
    if num_retries is None:
        return 3
    return max(num_retries, 1)


def backoff_seconds(attempt: int) -> float:
    seconds = 2.0 ** (attempt - 1)
    return min(seconds, 10.0)


def retry_kwargs(kwargs: dict[str, CallValue], retry_strategy: str | None) -> bool:
    # the inner call must not retry again, and only exponential_backoff_retry waits between attempts
    kwargs["num_retries"] = 0
    kwargs["max_retries"] = 0
    return retry_strategy == "exponential_backoff_retry"


def call_default(
    *args: JsonValue, **kwargs: CallValue
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    arguments = call_arguments_from(args, kwargs)
    return completion_from_arguments(arguments)


def completion_with_retries(
    *args: JsonValue,
    original_function: RetryCall | None = None,
    num_retries: int | None = None,
    retry_strategy: str | None = None,
    **kwargs: CallValue,
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    function: RetryCall = call_default if original_function is None else original_function
    attempts = attempts_for(num_retries)
    exponential = retry_kwargs(kwargs, retry_strategy)
    for attempt in range(1, attempts):
        try:
            return function(*args, **kwargs)
        except Exception:
            if exponential:
                time.sleep(backoff_seconds(attempt))
    return function(*args, **kwargs)


async def acompletion_with_retries(
    *args: JsonValue,
    original_function: RetryCall | None = None,
    num_retries: int | None = None,
    retry_strategy: str | None = None,
    **kwargs: CallValue,
) -> CompletionOutcome:
    function: RetryCall = call_default if original_function is None else original_function
    attempts = attempts_for(num_retries)
    exponential = retry_kwargs(kwargs, retry_strategy)
    remaining = attempts - 1
    while remaining > 0:
        try:
            return await settle_outcome(function(*args, **kwargs))
        except Exception:
            if exponential:
                await asyncio.sleep(backoff_seconds(attempts - remaining))
            remaining -= 1
    return await settle_outcome(function(*args, **kwargs))
