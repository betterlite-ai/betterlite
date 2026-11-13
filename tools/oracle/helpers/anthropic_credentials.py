import os
from collections.abc import Callable
from typing import Final, NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
from litellm.llms.anthropic.common_utils import (
    AnthropicModelInfo,
    is_anthropic_oauth_key,
    optionally_handle_anthropic_oauth,  # pyright: ignore[reportUnknownVariableType]
    resolve_used_client_oauth_token,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Header(TypedDict):
    name: str
    value: str


class Reply(TypedDict):
    status: int
    body: str


class Input(TypedDict):
    function: str
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    use_bearer_for_custom_base: NotRequired[bool]
    headers: NotRequired[list[Header]]
    environment: NotRequired[dict[str, str]]
    client_sent_oauth_token: NotRequired[Json]
    provider: NotRequired[str | None]
    model: NotRequired[str | None]
    value: NotRequired[str | None]
    reply: NotRequired[Reply]


class FixtureClient:
    def __init__(self, reply: Reply | None) -> None:
        self.reply = reply
        self.url: str | None = None
        self.headers: dict[str, str] = {}

    def get(self, url: str, headers: dict[str, str]) -> httpx.Response:
        if self.reply is None:
            raise RuntimeError("the models fetch reached the HTTP client without a fixture reply")
        self.url = url
        self.headers = headers
        return httpx.Response(
            self.reply["status"],
            content=self.reply["body"].encode(),
            request=httpx.Request("GET", url),
        )


def auth_header(argument: Input) -> dict[str, Json]:
    header = cast(
        "Json",
        AnthropicModelInfo.get_auth_header(  # pyright: ignore[reportUnknownMemberType]
            argument.get("api_key"),
            argument.get("api_base"),
            argument.get("use_bearer_for_custom_base", False),
        ),
    )
    return {"header": header}


def api_base(argument: Input) -> dict[str, Json]:
    return {"api_base": AnthropicModelInfo.get_api_base(argument.get("api_base"))}


def oauth_key(argument: Input) -> dict[str, Json]:
    return {"oauth": is_anthropic_oauth_key(argument.get("value"))}


def used_oauth_token(argument: Input) -> dict[str, Json]:
    return {
        "used": resolve_used_client_oauth_token(
            argument.get("client_sent_oauth_token"),
            argument.get("provider"),
        )
    }


def handle_oauth(argument: Input) -> dict[str, Json]:
    headers: dict[str, str] = {
        header["name"]: header["value"] for header in argument.get("headers", [])
    }
    updated, resolved = optionally_handle_anthropic_oauth(  # pyright: ignore[reportUnknownVariableType]
        headers, argument.get("api_key")
    )
    return {"headers": cast("Json", updated), "api_key": resolved}


def api_key_header(argument: Input) -> dict[str, Json]:
    api_key = argument.get("api_key")
    if api_key is None:
        raise ValueError("the api_key_header case needs an api_key")
    header = cast(
        "Json",
        AnthropicModelInfo._make_api_key_auth_header(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            api_key,
            argument.get("api_base"),
            argument.get("use_bearer_for_custom_base", False),
        ),
    )
    return {"header": header}


def base_model(argument: Input) -> dict[str, Json]:
    return {"model": AnthropicModelInfo.get_base_model(argument.get("model"))}


def models(argument: Input) -> dict[str, Json]:
    client = FixtureClient(argument.get("reply"))
    with mock.patch.object(litellm, "module_level_client", client):
        names = AnthropicModelInfo().get_models(
            api_key=argument.get("api_key"),
            api_base=argument.get("api_base"),
        )
    return {
        "url": client.url,
        "headers": cast("Json", client.headers),
        "models": cast("Json", names),
    }


HANDLERS: Final[dict[str, Callable[[Input], dict[str, Json]]]] = {
    "auth_header": auth_header,
    "api_base": api_base,
    "oauth_key": oauth_key,
    "used_oauth_token": used_oauth_token,
    "handle_oauth": handle_oauth,
    "api_key_header": api_key_header,
    "base_model": base_model,
    "models": models,
}


def run(argument: Input) -> dict[str, Json]:
    handler = HANDLERS.get(argument["function"])
    if handler is None:
        raise ValueError(f"unknown function {argument['function']}")
    with mock.patch.dict(os.environ, argument.get("environment", {}), clear=True):
        return handler(argument)
