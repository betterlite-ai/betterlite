from collections.abc import Awaitable, Callable
from typing import Final, Protocol, TypeAlias

from pydantic import JsonValue

import betterlite
from betterlite._completion.acompletion import acompletion
from betterlite._completion.arguments import JSON, messages_value
from betterlite._completion.completion import CompletionOutcome, completion
from betterlite.types.utils import Message

_CompletionCall: TypeAlias = Callable[..., CompletionOutcome | Awaitable[CompletionOutcome]]
_AsyncCall: TypeAlias = Callable[..., Awaitable[CompletionOutcome]]

_COMPLETION: Final[_CompletionCall] = completion
_ACOMPLETION: Final[_AsyncCall] = acompletion


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
    match model:
        case str() if len(model) > 0:
            return model
        case object():
            configured = params.get("model")
            match configured:
                case str():
                    return configured
                case object():
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
        for name, value in kwargs.items():
            self.params[name] = value
        selected = _selected_model(model, self.params)
        if self.router_obj is not None:
            return self.router_obj.completion(model=selected, messages=messages, **self.params)
        forwarded: dict[str, JsonValue] = dict(self.params)
        forwarded["model"] = selected
        forwarded["messages"] = messages_value(messages)
        return _COMPLETION(**forwarded)


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
        for name, value in kwargs.items():
            self.params[name] = value
        selected = _selected_model(model, self.params)
        if self.router_obj is not None:
            return await self.router_obj.acompletion(
                model=selected, messages=messages, **self.params
            )
        forwarded: dict[str, JsonValue] = dict(self.params)
        forwarded["model"] = selected
        forwarded["messages"] = messages_value(messages)
        return await _ACOMPLETION(**forwarded)


class Chat:
    params: dict[str, JsonValue]
    completions: AsyncCompletions | Completions

    def __init__(self, params: dict[str, JsonValue], router_obj: Router | None) -> None:
        self.params = params
        if self.params.get("acompletion") is True:
            self.params.pop("acompletion")
            self.completions = AsyncCompletions(self.params, router_obj=router_obj)
        else:
            self.completions = Completions(self.params, router_obj=router_obj)


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
        retries: JsonValue = max_retries
        if retries is None:
            retries = JSON.validate_python(betterlite.num_retries)
        params: dict[str, JsonValue] = {
            "api_key": api_key,
            "organization": organization,
            "base_url": base_url,
            "timeout": timeout,
            "max_retries": retries,
            "default_headers": default_headers,
        }
        self.params = params
        self.chat = Chat(params, router_obj=None)
