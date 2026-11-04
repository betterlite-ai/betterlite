import asyncio

# ruff: noqa: INP001 - websocket_oracle.py supplies this service namespace via __path__.
from collections.abc import Callable
from typing import Protocol, TypedDict, cast

from litellm.llms.openai.realtime.handler import OpenAIRealtime


class Input(TypedDict, total=False):
    model: str
    api_base: str
    api_key: str | None
    query_params: dict[str, str] | None
    openai_beta_realtime: bool


class Headers(Protocol):
    def __call__(self, api_key: str, *, openai_beta_realtime: bool) -> dict[str, str]: ...


class Realtime(Protocol):
    async def async_realtime(
        self, model: str, websocket: object, logging_obj: object,
        *, api_base: str, api_key: str | None,
    ) -> None: ...


def run(input_value: Input) -> object:
    handler = cast(Realtime, cast(object, OpenAIRealtime()))
    model = input_value.get("model", "realtime-a")
    base = input_value.get("api_base", "https://api.openai.com/")
    key = input_value.get("api_key", "sk-test")
    if key is None:
        asyncio.run(handler.async_realtime(model, None, None, api_base=base, api_key=None))
        raise ValueError("missing-key call unexpectedly succeeded")
    supplied = input_value.get("query_params")
    query = {"model": model} if supplied is None else supplied
    url_method = "_construct_url"
    headers_method = "_get_additional_headers"
    construct_url = cast(Callable[[str, dict[str, str]], str], getattr(handler, url_method))
    headers = cast(Headers, getattr(handler, headers_method))
    return {
        "url": construct_url(base, query),
        "query": query,
        "headers": headers(
            key, openai_beta_realtime=input_value.get("openai_beta_realtime", False)
        ),
    }
