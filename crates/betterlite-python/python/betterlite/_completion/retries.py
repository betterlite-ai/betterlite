import asyncio
import time
from collections.abc import Awaitable, Callable
from typing import TypeAlias

from pydantic import JsonValue

from betterlite._completion.arguments import CallValue
from betterlite._completion.completion import completion, settle_outcome
from betterlite._completion.stream import CompletionOutcome

RetryCall: TypeAlias = Callable[..., CompletionOutcome | Awaitable[CompletionOutcome]]


def attempts_for(num_retries: int | None) -> int:
    if num_retries is None:
        return 3
    return max(num_retries, 1)


def backoff_seconds(attempt: int) -> float:
    return min(2.0 ** (attempt - 1), 10.0)


def retry_kwargs(kwargs: dict[str, CallValue], retry_strategy: str | None) -> bool:
    # the inner call must not retry again, and only exponential_backoff_retry waits between attempts
    kwargs["num_retries"] = 0
    kwargs["max_retries"] = 0
    return retry_strategy == "exponential_backoff_retry"


def completion_with_retries(
    *args: JsonValue,
    original_function: RetryCall | None = None,
    num_retries: int | None = None,
    retry_strategy: str | None = None,
    **kwargs: CallValue,
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    function: RetryCall = completion if original_function is None else original_function
    attempts = attempts_for(num_retries)
    exponential = retry_kwargs(kwargs, retry_strategy)
    for attempt in range(1, attempts + 1):
        try:
            return function(*args, **kwargs)
        except Exception:
            if attempt == attempts:
                raise
            if exponential:
                time.sleep(backoff_seconds(attempt))


async def acompletion_with_retries(
    *args: JsonValue,
    original_function: RetryCall | None = None,
    num_retries: int | None = None,
    retry_strategy: str | None = None,
    **kwargs: CallValue,
) -> CompletionOutcome:
    function: RetryCall = completion if original_function is None else original_function
    attempts = attempts_for(num_retries)
    exponential = retry_kwargs(kwargs, retry_strategy)
    remaining = attempts
    while remaining > 0:
        remaining -= 1
        try:
            # a sync original_function answers with an outcome, which settle_outcome keeps as it is
            return await settle_outcome(function(*args, **kwargs))
        except Exception:
            if remaining == 0:
                raise
            if exponential:
                await asyncio.sleep(backoff_seconds(attempts - remaining))
