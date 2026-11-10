import logging
import os
import time
from collections.abc import Generator
from contextlib import contextmanager
from typing import Self, TypeAlias, TypedDict, cast
from unittest import mock

import azure.identity
import httpx
from azure.core.exceptions import ClientAuthenticationError
from azure.identity import ClientSecretCredential
from litellm.llms.azure.common_utils import (
    AzureOpenAIError,
    _cached_entra_id_token_provider,  # pyright: ignore[reportPrivateUsage]
    get_azure_ad_token_from_entra_id,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
FIXTURE_TENANT = "tenant"
FIXTURE_CLIENT = "client"
FIXTURE_VALUE = "secret"
HTTP_ERROR_STATUS = 400
IMDS_HOST = "169.254.169.254"
OPENID_PATH = ".well-known/openid-configuration"
NOW = 1_700_000_000


class ReplyRequired(TypedDict):
    status: int
    body: str


class Reply(ReplyRequired, total=False):
    url_contains: str
    error: str


class Call(TypedDict):
    tenant_id: str
    client_id: str
    client_secret: str


class Step(TypedDict, total=False):
    environment: dict[str, str]
    tenant_id: str
    client_id: str
    client_secret: str
    scope: str | None
    authority_host: str | None
    replies: list[Reply]
    advance: int
    calls: list[Call]
    status_code: int
    message: str
    headers: dict[str, str]
    request: dict[str, str]
    response: dict[str, int]
    body: Json


class Input(TypedDict):
    action: str
    steps: list[Step]


class RecordedRequest(TypedDict):
    method: str
    url: str
    query: dict[str, str]
    form: dict[str, str]
    headers: dict[str, str]


class FixtureResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.text = reply["body"]
        self.headers: dict[str, str] = {}

    def raise_for_status(self) -> None:
        if self.status_code >= HTTP_ERROR_STATUS:
            raise RuntimeError(f"HTTP Error: {self.status_code}")


class FixtureClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[RecordedRequest] = []

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def close(self) -> None:
        return None

    def get_error_response(self, _result: object) -> None:
        return None

    def _record(
        self,
        method: str,
        url: str,
        params: dict[str, object] | None,
        data: dict[str, object] | None,
        headers: dict[str, str] | None,
    ) -> None:
        self.requests.append(
            {
                "method": method,
                "url": url,
                "query": {name: str(value) for name, value in (params or {}).items()},
                "form": {name: str(value) for name, value in (data or {}).items()},
                "headers": dict(headers or {}),
            }
        )

    def _reply(self, url: str) -> FixtureResponse:
        for index, reply in enumerate(self.replies):
            needle = reply.get("url_contains")
            if needle is None or needle in url:
                matched = self.replies.pop(index)
                error = matched.get("error")
                if error is not None:
                    raise OSError(error)
                return FixtureResponse(matched)
        raise AssertionError(f"no reply for {url}")

    def get(
        self,
        url: str,
        params: dict[str, object] | None = None,
        headers: dict[str, str] | None = None,
        **_kwargs: object,
    ) -> FixtureResponse:
        self._record("GET", url, params, None, headers)
        return self._reply(url)

    def post(
        self,
        url: str,
        params: dict[str, object] | None = None,
        data: dict[str, object] | None = None,
        headers: dict[str, str] | None = None,
        **_kwargs: object,
    ) -> FixtureResponse:
        self._record("POST", url, params, data, headers)
        return self._reply(url)


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[tuple[int, str]] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append((record.levelno, record.getMessage()))


CAPTURE = LogCapture()
logging.getLogger("msal.application").addHandler(CAPTURE)
logging.getLogger("msal.application").setLevel(logging.DEBUG)


@contextmanager
def fixture_environment(environment: dict[str, str]) -> Generator[None]:
    with mock.patch.dict(os.environ, environment, clear=True):
        yield


@contextmanager
def fixed_clock() -> Generator[mock.MagicMock]:
    with mock.patch.object(time, "time", return_value=float(NOW)) as clock:
        yield clock


@contextmanager
def fixture_msal_client(client: FixtureClient) -> Generator[None]:
    with mock.patch(
        "azure.identity._internal.msal_credentials.MsalClient",
        return_value=client,
    ):
        yield


def environment_of(step: Step) -> dict[str, str]:
    return dict(step.get("environment", {}))


def grant_environment(step: Step) -> dict[str, str]:
    environment = environment_of(step)
    host = step.get("authority_host")
    if host is not None:
        environment["AZURE_AUTHORITY_HOST"] = host
    return environment


def scope_args(step: Step) -> tuple[str, ...]:
    scope = step.get("scope")
    return () if scope is None else (scope,)


def grant_step(step: Step) -> Json:
    _cached_entra_id_token_provider.cache_clear()
    client = FixtureClient(list(step.get("replies", [])))
    with (
        fixture_environment(grant_environment(step)),
        fixture_msal_client(client),
        fixed_clock(),
    ):
        try:
            provider = get_azure_ad_token_from_entra_id(
                step.get("tenant_id", ""),
                step.get("client_id", ""),
                step.get("client_secret", ""),
                *scope_args(step),
            )
            token = provider()
        except (ClientAuthenticationError, ValueError, RuntimeError, OSError):
            return {"kind": "error"}
    post = next((request for request in client.requests if request["method"] == "POST"), None)
    if post is None:
        return {"kind": "error"}
    return cast(
        "Json",
        {"kind": "token", "token_url": post["url"], "form": post["form"], "token": token},
    )


def expiry_attempt(credential: ClientSecretCredential, scope: str) -> Json:
    try:
        info = credential.get_token_info(scope)
    except (ClientAuthenticationError, ValueError, RuntimeError, OSError):
        return {"kind": "error"}
    return {
        "expires_on": info.expires_on,
        "token_type": info.token_type,
        "refresh_on": info.refresh_on,
    }


# one token request, then a second after the clock moved on when the step asks for it
def expiry_step(step: Step) -> Json:
    client = FixtureClient(list(step.get("replies", [])))
    with (
        fixture_environment(grant_environment(step)),
        fixture_msal_client(client),
        fixed_clock() as clock,
    ):
        credential = ClientSecretCredential(
            step.get("tenant_id", ""),
            step.get("client_id", ""),
            step.get("client_secret", ""),
        )
        scope = step.get("scope")
        scope = DEFAULT_SCOPE if scope is None else scope
        attempts = [expiry_attempt(credential, scope)]
        advance = step.get("advance")
        if advance is not None:
            clock.return_value = float(NOW + advance)
            attempts.append(expiry_attempt(credential, scope))
    return cast("Json", attempts)


class FakeCredential:
    def __init__(
        self, tenant_id: str, client_id: str, client_secret: str, **_kwargs: object
    ) -> None:
        self.tenant_id = tenant_id
        self.client_id = client_id
        self.client_secret = client_secret


def resolve_step(step: Step) -> Json:
    _cached_entra_id_token_provider.cache_clear()
    captured: list[tuple[str, str, str, str]] = []

    def fake_provider(credential: FakeCredential, *scopes: str) -> object:
        captured.append(
            (
                credential.tenant_id,
                credential.client_id,
                credential.client_secret,
                scopes[0] if scopes else "",
            )
        )
        return object()

    with (
        fixture_environment(environment_of(step)),
        mock.patch.object(azure.identity, "ClientSecretCredential", FakeCredential),
        mock.patch.object(azure.identity, "get_bearer_token_provider", fake_provider),
    ):
        try:
            get_azure_ad_token_from_entra_id(
                step.get("tenant_id", ""),
                step.get("client_id", ""),
                step.get("client_secret", ""),
                *scope_args(step),
            )
        except ValueError:
            return {"kind": "error"}
    if not captured:
        return {"kind": "error"}
    tenant_id, client_id, client_secret, scope = captured[0]
    return {
        "kind": "token",
        "tenant_id": tenant_id,
        "client_id": client_id,
        "client_secret": client_secret,
        "scope": scope,
    }


class CacheState:
    def __init__(self) -> None:
        self.constructed = 0


def cache_report(steps: list[Step]) -> list[dict[str, Json]]:
    _cached_entra_id_token_provider.cache_clear()
    state = CacheState()

    def fake_credential(
        _tenant_id: str, _client_id: str, _client_secret: str, **_kwargs: object
    ) -> FakeCredential:
        state.constructed += 1
        return FakeCredential("", "", "")

    def fake_provider(_credential: FakeCredential, *_scopes: str) -> object:
        return object()

    results: list[dict[str, Json]] = []
    with (
        mock.patch.object(azure.identity, "ClientSecretCredential", fake_credential),
        mock.patch.object(azure.identity, "get_bearer_token_provider", fake_provider),
    ):
        for step in steps:
            before = state.constructed
            with fixture_environment(step.get("environment", {})):
                providers = [
                    get_azure_ad_token_from_entra_id(
                        call["tenant_id"],
                        call["client_id"],
                        call["client_secret"],
                    )
                    for call in step.get("calls", [])
                ]
            results.append(
                {
                    "constructed": state.constructed - before,
                    "same": bool(providers) and providers[0] is providers[-1],
                }
            )
    return results


def error_step(step: Step) -> Json:
    request_fixture = step.get("request")
    request_object = (
        None
        if request_fixture is None
        else httpx.Request(request_fixture["method"], request_fixture["url"])
    )
    response_fixture = step.get("response")
    response_object = (
        None
        if response_fixture is None
        else httpx.Response(response_fixture["status_code"], request=request_object)
    )
    body = step.get("body")
    error = AzureOpenAIError(
        status_code=step.get("status_code", 0),
        message=step.get("message", ""),
        request=request_object,
        response=response_object,
        headers=step.get("headers"),
        body=body if isinstance(body, dict) else None,
    )
    return cast(
        "Json",
        {
            "status_code": error.status_code,
            "message": error.message,
            "request": {
                "method": error.request.method,
                "url": str(error.request.url),
            },
            "response": {
                "status_code": error.response.status_code,
                "request": {
                    "method": error.response.request.method,
                    "url": str(error.response.request.url),
                },
            },
            "headers": error.headers,
            "body": error.body,
            "status_code_is_synthesized": error.status_code_is_synthesized,
        },
    )


def configured_json(value: object) -> Json:
    if value is True:
        return "auto"
    if isinstance(value, str):
        return value
    return None


def warnings_of(messages: list[tuple[int, str]]) -> list[str]:
    return [message for level, message in messages if level == logging.WARNING]


def debug_of(messages: list[tuple[int, str]]) -> str | None:
    for _level, message in messages:
        if message.startswith("Region to be used:"):
            return message
    return None


def regional_host(requests: list[RecordedRequest], environment: dict[str, str]) -> str | None:
    hosts = [
        request["url"].split("://", maxsplit=1)[-1].split("/", maxsplit=1)[0]
        for request in requests
        if OPENID_PATH in request["url"]
    ]
    configured = environment.get("AZURE_AUTHORITY_HOST", "login.microsoftonline.com")
    central = configured.removeprefix("https://").rstrip(" /")
    if not hosts or hosts[-1] == central:
        return None
    return hosts[-1]


def region_of(host: str | None) -> str | None:
    if host is None:
        return None
    return host.split(".", maxsplit=1)[0]


def imds_reports(requests: list[RecordedRequest]) -> tuple[int, str | None]:
    imds = [request for request in requests if IMDS_HOST in request["url"]]
    metadata = imds[0]["headers"].get("Metadata") if imds else None
    return len(imds), metadata


def region_flow(step: Step) -> tuple[FixtureClient, dict[str, str], Json]:
    client = FixtureClient(list(step.get("replies", [])))
    environment = environment_of(step)
    CAPTURE.messages.clear()
    with (
        fixture_environment(environment),
        fixture_msal_client(client),
        fixed_clock(),
    ):
        credential = ClientSecretCredential(FIXTURE_TENANT, FIXTURE_CLIENT, FIXTURE_VALUE)
        configured = configured_json(
            credential._regional_authority  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
        )
        credential.get_token(DEFAULT_SCOPE)
    return client, environment, configured


def region_inputs_report(step: Step) -> Json:
    client, environment, configured = region_flow(step)
    host = regional_host(client.requests, environment)
    return cast(
        "Json",
        {
            "configured": configured,
            "region": region_of(host),
            "warnings": warnings_of(CAPTURE.messages),
            "debug": debug_of(CAPTURE.messages),
            "regional_host": host,
        },
    )


def region_discovery_report(step: Step) -> Json:
    client, environment, _configured = region_flow(step)
    host = regional_host(client.requests, environment)
    imds_requests, metadata = imds_reports(client.requests)
    return cast(
        "Json",
        {
            "region": region_of(host),
            "warnings": warnings_of(CAPTURE.messages),
            "debug": debug_of(CAPTURE.messages),
            "imds_requests": imds_requests,
            "metadata": metadata,
            "regional_host": host,
        },
    )


def region_host_report(step: Step) -> Json:
    client = FixtureClient(list(step.get("replies", [])))
    environment = environment_of(step)
    CAPTURE.messages.clear()
    outcome = "token"
    with (
        fixture_environment(environment),
        fixture_msal_client(client),
        fixed_clock(),
    ):
        try:
            credential = ClientSecretCredential(
                FIXTURE_TENANT, FIXTURE_CLIENT, FIXTURE_VALUE
            )
            credential.get_token(DEFAULT_SCOPE)
        except (ClientAuthenticationError, ValueError, RuntimeError, OSError):
            outcome = "error"
    return cast(
        "Json",
        {
            "outcome": outcome,
            "requests": [
                {
                    "method": request["method"],
                    "url": request["url"],
                    "query": request["query"],
                    "form": request["form"],
                }
                for request in client.requests
            ],
            "regional_host": regional_host(client.requests, environment),
        },
    )


def run(argument: Input) -> Json:
    if argument["action"] == "client_secret_provider_cache":
        return cast("Json", cache_report(argument["steps"]))
    if argument["action"] == "regional_authority_inputs":
        return [region_inputs_report(step) for step in argument["steps"]]
    return run_steps(argument)


def run_steps(argument: Input) -> Json:
    if argument["action"] == "client_secret_reference_resolution":
        return [resolve_step(step) for step in argument["steps"]]
    if argument["action"] == "token_expiry":
        return [expiry_step(step) for step in argument["steps"]]
    if argument["action"] == "azure_error_fields":
        return [error_step(step) for step in argument["steps"]]
    return run_regional(argument)


def run_regional(argument: Input) -> Json:
    if argument["action"] == "regional_authority_discovery":
        return [region_discovery_report(step) for step in argument["steps"]]
    if argument["action"] == "regional_authority_host":
        return [region_host_report(step) for step in argument["steps"]]
    return [grant_step(step) for step in argument["steps"]]
