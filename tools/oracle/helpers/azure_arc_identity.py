import io
import os
import sys
import time
from collections.abc import Generator
from contextlib import contextmanager
from importlib import import_module
from types import SimpleNamespace
from typing import Literal, Protocol, TypeAlias, TypedDict, cast
from unittest import mock

from azure.core.exceptions import ClientAuthenticationError
from azure.identity._credentials.azure_arc import AzureArcCredential
from litellm.secret_managers.get_azure_ad_token_provider import get_azure_ad_token_provider
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

CLIENT_ID = "fixture-client"


class MsalManagedIdentity(Protocol):
    def _get_arc_endpoint(self) -> str | None: ...


msal_managed_identity = cast("MsalManagedIdentity", import_module("msal.managed_identity"))


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Host(TypedDict):
    platform: str
    environment: dict[str, str]
    files: dict[str, str]
    now: int


class EndpointInput(Host):
    action: Literal["endpoint"]


class TokenInput(Host):
    action: Literal["token"]
    resource: str
    identity: Literal["system", "user"]
    responses: list[Reply]


Input: TypeAlias = EndpointInput | TokenInput


class FixtureResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.headers = reply["headers"]
        self.text = reply["body"]


class FixtureClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = [FixtureResponse(reply) for reply in replies]
        self.requests: list[Json] = []

    def get(
        self,
        url: str,
        params: dict[str, str] | None = None,
        headers: dict[str, str] | None = None,
    ) -> FixtureResponse:
        request: dict[str, Json] = {
            "url": url,
            "params": dict(params or {}),
            "headers": dict(headers or {}),
        }
        self.requests.append(request)
        return self.replies.pop(0)

    def post(self, url: str) -> FixtureResponse:
        raise AssertionError(url)

    def close(self) -> None:
        return None


class FixtureArcCredential(AzureArcCredential):
    def _create_http_client(self, **kwargs: object) -> object:
        return kwargs["http_client"]


@contextmanager
def fixture_host(argument: Host) -> Generator[None]:
    files = argument["files"]

    def stat(path: str) -> SimpleNamespace:
        if path not in files:
            raise FileNotFoundError(2, "No such file or directory", path)
        return SimpleNamespace(st_size=len(files[path].encode()))

    def open_text(path: str) -> io.TextIOWrapper:
        return io.TextIOWrapper(io.BytesIO(files[path].encode()))

    def exists(path: str) -> bool:
        return path in files

    real = os.path
    path_shim = SimpleNamespace(
        exists=exists,
        expandvars=real.expandvars,
        join=real.join,
        splitext=real.splitext,
        basename=real.basename,
    )
    with (
        mock.patch.dict(os.environ, argument["environment"], clear=True),
        mock.patch.object(sys, "platform", argument["platform"]),
        mock.patch.object(time, "time", return_value=float(argument["now"])),
        mock.patch.object(
            import_module("msal.managed_identity"),
            "os",
            SimpleNamespace(environ=os.environ, stat=stat, path=path_shim),
        ),
        mock.patch.object(import_module("msal.managed_identity"), "open", open_text, create=True),
    ):
        yield


def bearer(argument: TokenInput, scope: str) -> str | None:
    client = FixtureClient(argument["responses"])
    user = argument["identity"] == "user"

    def factory(client_id: str | None = None) -> FixtureArcCredential:
        return FixtureArcCredential(client_id=client_id, http_client=client)

    credential = (
        AzureCredentialType.ManagedIdentityCredential
        if user
        else AzureCredentialType.DeploymentIdentityCredential
    )
    variables = {"AZURE_CLIENT_ID": CLIENT_ID} if user else {}
    with (
        mock.patch.dict(os.environ, variables),
        mock.patch("azure.identity.ManagedIdentityCredential", factory),
    ):
        try:
            return get_azure_ad_token_provider(azure_scope=scope, azure_credential=credential)()
        except ClientAuthenticationError:
            return None


def acquire(argument: TokenInput) -> dict[str, Json]:
    if not msal_managed_identity._get_arc_endpoint():  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return {"requests": [], "result": {"kind": "no_endpoint"}, "bearer": None}
    scope = f"{argument['resource']}/.default"
    client = FixtureClient(argument["responses"])
    client_id = CLIENT_ID if argument["identity"] == "user" else None
    credential = FixtureArcCredential(client_id=client_id, http_client=client)
    result: dict[str, Json]
    try:
        token = credential.get_token_info(scope)
    except (ClientAuthenticationError, OSError, ValueError) as error:
        message = error.message if isinstance(error, ClientAuthenticationError) else str(error)
        result = {"kind": "error", "class": type(error).__name__, "message": message}
    else:
        result = {
            "kind": "token",
            "token": token.token,
            "expires_on": token.expires_on,
            "token_type": token.token_type,
            "refresh_on": token.refresh_on,
        }
    return {"requests": client.requests, "result": result, "bearer": bearer(argument, scope)}


def run(argument: Input) -> object:
    with fixture_host(argument):
        if argument["action"] == "endpoint":
            return {
                "endpoint": msal_managed_identity._get_arc_endpoint() or None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            }
        return acquire(argument)
