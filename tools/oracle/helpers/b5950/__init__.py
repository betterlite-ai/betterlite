import io
import json
import os
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime
from functools import partial
from pathlib import Path
from typing import TYPE_CHECKING, Self, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.sap import credentials as sap_credentials
from litellm.llms.sap.chat import transformation as sap_chat
from litellm.llms.sap.embed import transformation as sap_embed

if TYPE_CHECKING:
    from pydantic import JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

CREDENTIAL_NAMES = (
    "client_id", "client_secret", "auth_url", "base_url",
    "cert_file_path", "key_file_path", "cert_str", "key_str",
)
BASE_URL = "https://sap.example.com/v2"
AUTHORIZATION = "Bearer case-token"
RESOURCE_GROUP = "default"


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


@contextmanager
def patched_environment(input_value: dict[str, Json]) -> Generator[None, None, None]:
    environment = object_value(input_value.get("environment"), "environment")
    variables = {
        name: string_value(value, name)
        for name, value in object_value(environment.get("vars", {}), "vars").items()
    }
    files = {
        name: string_value(value, name)
        for name, value in object_value(environment.get("files", {}), "files").items()
    }
    home = string_value(environment.get("home"), "home")

    def exists(path: Path) -> bool:
        return str(path) in files

    def open_path(path: Path, *_args: object, **_kwargs: object) -> io.StringIO:
        data = files.get(str(path))
        if data is None:
            raise FileNotFoundError(str(path))
        return io.StringIO(data)

    with (
        patch.dict(os.environ, variables, clear=False),
        patch.object(sap_credentials, "DEFAULT_HOME_PATH", str(Path(home) / ".aicore")),
        patch.object(Path, "exists", exists),
        patch.object(Path, "open", open_path),
    ):
        yield


@dataclass
class Recorder:
    responses: list[str]
    requests: list[str]

    def next_response(self) -> str:
        index = min(len(self.requests), len(self.responses)) - 1
        if index < 0:
            raise ValueError("the case scripted no token response")
        return self.responses[index]


class FakeSapHandler:
    def __init__(self, recorder: Recorder, **_kwargs: object) -> None:
        self.recorder = recorder

    def post(
        self,
        url: str,
        data: dict[str, str] | None = None,
        **_kwargs: object,
    ) -> "JsonResponse":
        del data
        self.recorder.requests.append(url)
        return JsonResponse(cast(Json, json.loads(self.recorder.next_response())))


@contextmanager
def patched_token_transport(recorder: Recorder) -> Generator[None, None, None]:
    handler = partial(FakeSapHandler, recorder)
    with (
        patch.object(sap_credentials, "_get_httpx_client", handler),
        patch.object(sap_credentials, "HTTPHandler", handler),
        patch.object(httpx, "Client"),
    ):
        yield


class FixedNow:
    def __init__(self, holder: list[datetime]) -> None:
        self.holder = holder

    def now(self, *_args: object) -> datetime:
        return self.holder[0]


def credentials_output(input_value: dict[str, Json]) -> Json:
    service_key = input_value.get("service_key")
    settings = input_value.get("litellm_sap_service_key")
    with (
        patched_environment(input_value),
        patch.object(litellm, "sap_service_key", settings),
    ):
        result = sap_credentials.fetch_credentials(  # pyright: ignore[reportUnknownMemberType]
            service_key=cast("str | dict[str, JsonValue] | None", service_key)
        )
    fetched = cast(dict[str, Json], result)
    return {name: fetched.get(name) for name in (*CREDENTIAL_NAMES, "resource_group")}


def validate_output(input_value: dict[str, Json]) -> Json:
    values = object_value(input_value.get("credentials"), "credentials")
    fields = {
        name: None if values.get(name) is None else string_value(values.get(name), name)
        for name in CREDENTIAL_NAMES
    }
    sap_credentials.validate_credentials(**fields)  # pyright: ignore[reportUnknownMemberType,reportArgumentType]
    return None


def token_output(input_value: dict[str, Json]) -> Json:
    values = object_value(input_value.get("credentials"), "credentials")
    responses = [
        string_value(item, "responses")
        for item in array_value(input_value.get("responses"), "responses")
    ]
    seconds = [
        cast(int, item) for item in array_value(input_value.get("times"), "times")
    ]
    moments = [datetime.fromtimestamp(second, UTC) for second in seconds]
    recorder = Recorder(responses=responses, requests=[])
    holder = [moments[0] if moments else datetime(2024, 1, 1, tzinfo=UTC)]
    with (
        patched_token_transport(recorder),
        patch.object(sap_credentials, "datetime", FixedNow(holder)),
        patch.object(
            sap_credentials,
            "fetch_credentials",
            return_value={
                name: string_value(value, name) for name, value in values.items()
            },
        ),
    ):
        creator, _base_url, _resource_group = sap_credentials.get_token_creator()  # pyright: ignore[reportUnknownMemberType]
        tokens: list[Json] = []
        for moment in moments:
            holder[0] = moment
            tokens.append(creator())
    return {"tokens": tokens, "requests": len(recorder.requests)}


class JsonResponse:
    def __init__(self, value: Json) -> None:
        self.value = value

    def json(self) -> Json:
        return self.value


class ConfigurationSource:
    def __init__(
        self,
        listing: Json,
        configurations: dict[str, Json],
        *_args: object,
        **_kwargs: object,
    ) -> None:
        self.listing = listing
        self.configurations = configurations

    def get(self, url: str, **_kwargs: object) -> JsonResponse:
        if url == BASE_URL + "/lm/deployments":
            return JsonResponse(self.listing)
        prefix = BASE_URL + "/lm/configurations/"
        if url.startswith(prefix):
            key = url[len(prefix) :]
            if key in self.configurations:
                return JsonResponse(self.configurations[key])
        raise KeyError(url)

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_args: object) -> None:
        return None


class TokenCreator:
    def __init__(self, token: str) -> None:
        self.token = token

    def __call__(self) -> str:
        return self.token


def fixed_token_creator(
    token: str,
    base_url: str,
    resource_group: str,
    *_args: object,
) -> tuple[TokenCreator, str, str]:
    return TokenCreator(token), base_url, resource_group


def chat_deployment_url(listing: Json, configurations: dict[str, Json]) -> Json:
    creator = partial(fixed_token_creator, AUTHORIZATION, BASE_URL, RESOURCE_GROUP)
    source = ConfigurationSource(listing, configurations)
    with (
        patch.object(litellm, "module_level_client", source),
        patch.object(sap_chat, "get_token_creator", creator),
    ):
        config = sap_chat.GenAIHubOrchestrationConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
        config.run_env_setup()  # pyright: ignore[reportUnknownMemberType]
        return cast(Json, config.deployment_url)  # pyright: ignore[reportUnknownMemberType]


def embed_deployment_url(listing: Json, configurations: dict[str, Json]) -> Json:
    creator = partial(fixed_token_creator, AUTHORIZATION, BASE_URL, RESOURCE_GROUP)
    client = partial(ConfigurationSource, listing, configurations)
    with (
        patch.object(sap_embed, "get_token_creator", creator),
        patch.object(httpx, "Client", client),
    ):
        config = sap_embed.GenAIHubEmbeddingConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
        return cast(Json, config.deployment_url)  # pyright: ignore[reportUnknownMemberType]


def deployment_output(input_value: dict[str, Json]) -> Json:
    mode = string_value(input_value.get("mode"), "mode")
    listing = input_value.get("listing")
    configurations = object_value(input_value.get("configurations"), "configurations")
    if mode == "chat":
        return chat_deployment_url(listing, configurations)
    if mode == "embedding":
        return embed_deployment_url(listing, configurations)
    raise ValueError(f"unknown mode {mode}")


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    if function == "credentials":
        return credentials_output(input_value)
    if function == "validate":
        return validate_output(input_value)
    if function == "token":
        return token_output(input_value)
    if function == "deployment":
        return deployment_output(input_value)
    raise ValueError(f"unknown function {function}")
