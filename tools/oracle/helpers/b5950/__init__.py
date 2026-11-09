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

BASE_URL = "https://sap.example.com/v2"
AUTHORIZATION = "Bearer case-token"
RESOURCE_GROUP = "default"
DEFAULT_RESPONSE_BODY = '{"access_token": "case-token", "expires_in": 3600}'
EPOCH = datetime(2024, 1, 1, tzinfo=UTC)


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


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_string(values: dict[str, Json], name: str) -> str | None:
    value = values.get(name)
    if value is None:
        return None
    return string_value(value, name)


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
    default_home = str(Path(home) / ".aicore")

    def exists(path: Path) -> bool:
        return str(path) in files

    def open_path(path: Path, *_args: object, **_kwargs: object) -> io.StringIO:
        data = files.get(str(path))
        if data is None:
            raise FileNotFoundError(str(path))
        return io.StringIO(data)

    with (
        patch.dict(os.environ, variables, clear=False),
        patch.object(sap_credentials, "DEFAULT_HOME_PATH", default_home),
        patch.object(Path, "exists", exists),
        patch.object(Path, "open", open_path),
    ):
        yield


@dataclass
class RecordedRequest:
    url: str
    form: dict[str, str]
    cert: tuple[str, str] | None
    cert_text: str | None
    key_text: str | None


@dataclass
class Recorder:
    responses: list[str]
    requests: list[RecordedRequest]

    def next_response(self) -> str:
        index = min(len(self.requests), len(self.responses)) - 1
        if index < 0:
            raise ValueError("the case scripted no token response")
        return self.responses[index]


class FakeResponse:
    def __init__(self, body: str) -> None:
        self.body = body

    def json(self) -> Json:
        return cast(Json, json.loads(self.body))


class FakeSapClient:
    def __init__(self, cert: tuple[str, str] | None = None, **_kwargs: object) -> None:
        self.cert = cert

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_args: object) -> None:
        return None


class FakeSapHandler:
    def __init__(self, recorder: Recorder, client: FakeSapClient | None = None) -> None:
        self.recorder = recorder
        self.client = client

    def post(
        self,
        url: str,
        data: dict[str, str] | None = None,
        timeout: float | None = None,
    ) -> FakeResponse:
        cert = None if self.client is None else self.client.cert
        cert_text = None
        key_text = None
        if cert is not None:
            cert_text = Path(cert[0]).read_text(encoding="utf-8")
            key_text = Path(cert[1]).read_text(encoding="utf-8")
        del timeout
        self.recorder.requests.append(
            RecordedRequest(
                url=url,
                form=dict(data or {}),
                cert=cert,
                cert_text=cert_text,
                key_text=key_text,
            )
        )
        return FakeResponse(self.recorder.next_response())


@contextmanager
def patched_token_transport(recorder: Recorder) -> Generator[None, None, None]:
    handler = partial(FakeSapHandler, recorder)
    with (
        patch.object(sap_credentials, "_get_httpx_client", handler),
        patch.object(sap_credentials, "HTTPHandler", handler),
        patch.object(httpx, "Client", FakeSapClient),
    ):
        yield


class FixedNow:
    def __init__(self, holder: list[datetime]) -> None:
        self.holder = holder

    def now(self, *_args: object) -> datetime:
        return self.holder[0]


def resolved_credentials(values: dict[str, Json]) -> dict[str, str]:
    return {name: string_value(value, name) for name, value in values.items()}


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
    return cast(Json, result)


def validate_output(input_value: dict[str, Json]) -> Json:
    values = object_value(input_value.get("credentials"), "credentials")
    sap_credentials.validate_credentials(  # pyright: ignore[reportUnknownMemberType]
        auth_url=optional_string(values, "auth_url"),
        base_url=optional_string(values, "base_url"),
        client_id=optional_string(values, "client_id"),
        client_secret=optional_string(values, "client_secret"),
        cert_str=optional_string(values, "cert_str"),
        key_str=optional_string(values, "key_str"),
        cert_file_path=optional_string(values, "cert_file_path"),
        key_file_path=optional_string(values, "key_file_path"),
    )
    return None


def identity_output(request: RecordedRequest, values: dict[str, Json]) -> Json:
    if request.cert is None:
        return {"kind": "none"}
    cert_path = optional_string(values, "cert_file_path")
    if cert_path is not None and request.cert[0] == cert_path:
        return {"kind": "files", "cert": request.cert[0], "key": request.cert[1]}
    return {
        "kind": "pem",
        "cert": request.cert_text if request.cert_text is not None else "",
        "key": request.key_text if request.key_text is not None else "",
    }


def token_request_output(input_value: dict[str, Json]) -> Json:
    values = object_value(input_value.get("credentials"), "credentials")
    recorder = Recorder(responses=[DEFAULT_RESPONSE_BODY], requests=[])
    holder = [EPOCH]
    with (
        patched_token_transport(recorder),
        patch.object(sap_credentials, "datetime", FixedNow(holder)),
        patch.object(
            sap_credentials,
            "fetch_credentials",
            return_value=resolved_credentials(values),
        ),
    ):
        creator, _base_url, _resource_group = sap_credentials.get_token_creator()  # pyright: ignore[reportUnknownMemberType]
        _token = creator()
    request = recorder.requests[0]
    return {
        "url": request.url,
        "form": cast(Json, request.form),
        "identity": identity_output(request, values),
    }


def token_output(input_value: dict[str, Json]) -> Json:
    values = object_value(input_value.get("credentials"), "credentials")
    responses = [
        string_value(item, "responses")
        for item in array_value(input_value.get("responses"), "responses")
    ]
    seconds = [
        integer_value(item, "times")
        for item in array_value(input_value.get("times"), "times")
    ]
    moments = [datetime.fromtimestamp(second, UTC) for second in seconds]
    recorder = Recorder(responses=responses, requests=[])
    holder = [moments[0] if moments else EPOCH]
    with (
        patched_token_transport(recorder),
        patch.object(sap_credentials, "datetime", FixedNow(holder)),
        patch.object(
            sap_credentials,
            "fetch_credentials",
            return_value=resolved_credentials(values),
        ),
    ):
        creator, _base_url, _resource_group = sap_credentials.get_token_creator()  # pyright: ignore[reportUnknownMemberType]
        tokens: list[Json] = []
        for moment in moments:
            holder[0] = moment
            tokens.append(creator())
    return {"tokens": tokens, "requests": len(recorder.requests)}


class DeploymentResponse:
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

    def get(self, url: str, **_kwargs: object) -> DeploymentResponse:
        if url == BASE_URL + "/lm/deployments":
            return DeploymentResponse(self.listing)
        prefix = BASE_URL + "/lm/configurations/"
        if url.startswith(prefix):
            key = url[len(prefix) :]
            if key in self.configurations:
                return DeploymentResponse(self.configurations[key])
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
    configurations = object_value(
        input_value.get("configurations"),
        "configurations",
    )
    if mode == "chat":
        return chat_deployment_url(listing, configurations)
    if mode == "embedding":
        return embed_deployment_url(listing, configurations)
    raise ValueError(f"unknown mode {mode}")


def headers_output(input_value: dict[str, Json]) -> Json:
    token = string_value(input_value.get("token"), "token")
    resource_group = string_value(input_value.get("resource_group"), "resource_group")
    creator = partial(fixed_token_creator, token, BASE_URL, resource_group)
    with patch.object(sap_chat, "get_token_creator", creator):
        config = sap_chat.GenAIHubOrchestrationConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
        config.run_env_setup()  # pyright: ignore[reportUnknownMemberType]
        headers = config.headers  # pyright: ignore[reportUnknownMemberType]
    return {name.lower(): value for name, value in headers.items()}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    if function == "credentials":
        return credentials_output(input_value)
    if function == "validate":
        return validate_output(input_value)
    if function == "token_request":
        return token_request_output(input_value)
    if function == "token":
        return token_output(input_value)
    if function == "deployment":
        return deployment_output(input_value)
    if function == "headers":
        return headers_output(input_value)
    raise ValueError(f"unknown function {function}")
