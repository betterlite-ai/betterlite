from collections.abc import Awaitable
from typing import Protocol

from pydantic import JsonValue

import betterlite
from betterlite._completion import arguments
from betterlite._completion.arguments import JSON, messages_value
from betterlite._completion.completion import completion_from_arguments, settle_outcome
from betterlite._completion.stream import CompletionOutcome
from betterlite.types.utils import Message


class Router(Protocol):
    def completion(
        self,
        model: str | None,
        messages: list[dict[str, JsonValue] | Message],
        **params: JsonValue,
    ) -> CompletionOutcome: ...

    def acompletion(
        self,
        model: str | None,
        messages: list[dict[str, JsonValue] | Message],
        **params: JsonValue,
    ) -> Awaitable[CompletionOutcome]: ...


def _selected_model(model: str | None, params: dict[str, JsonValue]) -> str | None:
    # LiteLLM's `model or self.params.get("model")`, so an empty name falls through to the params
    if model is not None and len(model) > 0:
        return model
    configured = params.get("model")
    match configured:
        case str():
            return configured
    return None


class Completions:
    params: dict[str, JsonValue]
    router_obj: Router | None

    def __init__(self, params: dict[str, JsonValue], router_obj: Router | None) -> None:
        self.params = params
        self.router_obj = router_obj

    def create(
        self,
        messages: list[dict[str, JsonValue] | Message],
        model: str | None = None,
        **kwargs: JsonValue,
    ) -> CompletionOutcome | Awaitable[CompletionOutcome]:
        self.params.update(kwargs)
        selected = _selected_model(model, self.params)
        forwarded = {**self.params, "model": selected, "messages": messages_value(messages)}
        if self.router_obj is not None:
            return self.router_obj.completion(model=selected, messages=messages, **self.params)
        return completion_from_arguments(forwarded)


class AsyncCompletions:
    params: dict[str, JsonValue]
    router_obj: Router | None

    def __init__(self, params: dict[str, JsonValue], router_obj: Router | None) -> None:
        self.params = params
        self.router_obj = router_obj

    async def create(
        self,
        messages: list[dict[str, JsonValue] | Message],
        model: str | None = None,
        **kwargs: JsonValue,
    ) -> CompletionOutcome:
        self.params.update(kwargs)
        chosen = _selected_model(model, self.params)
        if self.router_obj is None:
            forwarded = {**self.params, "model": chosen, "messages": messages_value(messages)}
            return await settle_outcome(
                completion_from_arguments({**forwarded, "acompletion": True})
            )
        return await self.router_obj.acompletion(
            model=chosen, messages=messages, **self.params
        )


class Chat:
    params: dict[str, JsonValue]
    completions: AsyncCompletions | Completions

    def __init__(self, params: dict[str, JsonValue], router_obj: Router | None) -> None:
        self.params = params
        if self.params.get("acompletion") is True:
            self.params.pop("acompletion")
            self.completions = AsyncCompletions(params, router_obj=router_obj)
        else:
            self.completions = Completions(params, router_obj=router_obj)


class LiteLLM:
    params: dict[str, JsonValue]
    chat: Chat

    def __init__(
        self,
        *,
        api_key: JsonValue = None,
        organization: JsonValue = None,
        base_url: JsonValue = None,
        timeout: JsonValue = 600,
        max_retries: JsonValue = None,
        default_headers: dict[str, JsonValue] | None = None,
    ) -> None:
        global_retries: JsonValue = betterlite.num_retries
        configured = arguments.num_retries if global_retries is None else global_retries
        retries = max_retries if max_retries is not None else JSON.validate_python(configured)
        self.params = {
            "api_key": api_key,
            "organization": organization,
            "base_url": base_url,
            "timeout": timeout,
            "max_retries": retries,
            "default_headers": default_headers,
        }
        self.chat = Chat(self.params, router_obj=None)
