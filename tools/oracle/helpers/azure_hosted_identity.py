import json
import os
import sys
import time
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from importlib import import_module
from types import SimpleNamespace
from typing import Literal, Protocol, TypeAlias, TypedDict, cast
from unittest import mock
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

from azure.core.exceptions import ClientAuthenticationError, DecodeError
from azure.core.rest import HttpRequest
from azure.identity import CredentialUnavailableError
from azure.identity._credentials.app_service import AppServiceCredential
from azure.identity._credentials.azure_ml import AzureMLCredential
from azure.identity._credentials.cloud_shell import CloudShellCredential
from azure.identity._credentials.service_fabric import ServiceFabricCredential
from azure.identity._internal import within_dac
from litellm.secret_managers.get_azure_ad_token_provider import get_azure_ad_token_provider
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

KEPT_HEADERS = frozenset(("metadata", "x-identity-header", "secret", "content-type"))


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class HostStep(TypedDict, total=False):
    environment: dict[str, str]
    platform: str
    files: dict[str, str]
    now: int


class TokenStep(HostStep):
    action: Literal["token"]
    protocol: Literal["app_service", "azure_ml", "service_fabric", "cloud_shell"]
    client_id: str
    identity_config: dict[str, str]
    within_default_credential: bool
    resource: str
    replies: list[Reply]


class SelectStep(HostStep):
    action: Literal["select"]


Step: TypeAlias = TokenStep | SelectStep


class Input(TypedDict):
    steps: list[Step]


FixtureCredential: TypeAlias = (
    AppServiceCredential | AzureMLCredential | CloudShellCredential | ServiceFabricCredential
)


class MsalModule(Protocol):
    SERVICE_FABRIC: object
    APP_SERVICE: object
    AZURE_ARC: object
    CLOUD_SHELL: object
    MACHINE_LEARNING: object

    def get_managed_identity_source(self) -> object: ...


msal_managed_identity = cast(MsalModule, import_module("msal.managed_identity"))


class FixtureResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.headers = dict(reply["headers"])
        self.reason = ""
        self.content_type = next(
            (value for name, value in reply["headers"].items() if name.lower() == "content-type"),
            None,
        )
        self._body = reply["body"].encode()

    def text(self, encoding: str | None = None) -> str:
        return self._body.decode("utf-8-sig" if encoding in (None, "utf-8") else encoding)


def recorded_request(
    method: str,
    url: str,
    headers: Mapping[str, str],
    form: Mapping[str, str] | None,
) -> dict[str, Json]:
    parts = urlsplit(url)
    query: dict[str, Json] = dict(parse_qsl(parts.query, keep_blank_values=True))
    kept: dict[str, Json] = {
        name.lower(): value for name, value in headers.items() if name.lower() in KEPT_HEADERS
    }
    return {
        "method": method,
        "url": urlunsplit((parts.scheme, parts.netloc, parts.path, "", "")),
        "params": query,
        "headers": kept,
        "form": None if form is None else dict(form),
    }


class ServiceFabricResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.headers = dict(reply["headers"])
        self.text = reply["body"]


class FixtureTransport:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[dict[str, Json]] = []

    def get(
        self, url: str, *, params: Mapping[str, str], headers: Mapping[str, str]
    ) -> ServiceFabricResponse:
        separator = "&" if urlsplit(url).query else "?"
        full_url = f"{url}{separator}{urlencode(params)}"
        self.requests.append(recorded_request("GET", full_url, headers, None))
        return ServiceFabricResponse(self.replies.pop(0))

    def send(self, request: HttpRequest, **_kwargs: object) -> FixtureResponse:
        self.requests.append(
            recorded_request(
                request.method,
                request.url,
                request.headers,
                cast("Mapping[str, str] | None", request.content),
            )
        )
        return FixtureResponse(self.replies.pop(0))


class FixtureFactory:
    def __init__(self, value: object) -> None:
        self.value = value

    def __call__(self, *_args: object, **_kwargs: object) -> object:
        return self.value


@contextmanager
def fixture_host(step: HostStep) -> Generator[None]:
    environment = step.get("environment") or {}
    files = step.get("files") or {}
    platform = step.get("platform") or "linux"
    now = step.get("now") or 0

    def exists(path: str) -> bool:
        return path in files

    path_shim = SimpleNamespace(exists=exists, expandvars=os.path.expandvars)
    msal_os = SimpleNamespace(environ=os.environ, path=path_shim, getenv=os.environ.get)
    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch.object(sys, "platform", platform),
        mock.patch.object(time, "time", return_value=float(now)),
        mock.patch.object(msal_managed_identity, "os", msal_os),
    ):
        yield


def category(error: BaseException) -> str:
    if isinstance(error, CredentialUnavailableError):
        return "unavailable"
    if isinstance(error, ClientAuthenticationError):
        return "client_authentication"
    if isinstance(error, (DecodeError, json.JSONDecodeError)):
        return "invalid_json"
    if isinstance(error, KeyError):
        missing = cast(object, error.args[0]) if error.args else "field"
        return "missing_" + str(missing)
    if isinstance(error, ValueError):
        return "invalid_value"
    return "transport"


def source_name(source: object) -> str:
    names = {
        msal_managed_identity.SERVICE_FABRIC: "service_fabric",
        msal_managed_identity.APP_SERVICE: "app_service",
        msal_managed_identity.MACHINE_LEARNING: "azure_ml",
        msal_managed_identity.AZURE_ARC: "arc",
        msal_managed_identity.CLOUD_SHELL: "cloud_shell",
    }
    return names.get(source, "vm")


def build_credential(step: TokenStep, transport: FixtureTransport) -> FixtureCredential:
    client_id = step.get("client_id")
    identity_config = step.get("identity_config") or None
    protocol = step["protocol"]
    if protocol == "cloud_shell":
        return CloudShellCredential(
            transport=transport, client_id=client_id, identity_config=identity_config
        )
    if protocol == "service_fabric":
        return ServiceFabricCredential(client_id=client_id, identity_config=identity_config)
    if protocol == "azure_ml":
        return AzureMLCredential(
            client_id=client_id, identity_config=identity_config, transport=transport
        )
    return AppServiceCredential(
        client_id=client_id, identity_config=identity_config, transport=transport
    )


def litellm_bearer(credential: FixtureCredential, scope: str) -> str | None:
    with mock.patch("azure.identity.ManagedIdentityCredential", FixtureFactory(credential)):
        provider = get_azure_ad_token_provider(
            azure_scope=scope,
            azure_credential=AzureCredentialType.DeploymentIdentityCredential,
        )
        try:
            return provider()
        except (ClientAuthenticationError, ValueError):
            return None


@contextmanager
def default_credential_scope(*, enabled: bool) -> Generator[None]:
    marker = within_dac.set(enabled)
    try:
        yield
    finally:
        within_dac.reset(marker)


def credential_token(step: TokenStep, transport: FixtureTransport, scope: str) -> dict[str, Json]:
    try:
        with default_credential_scope(enabled=step.get("within_default_credential", False)):
            credential = build_credential(step, transport)
            token = credential.get_token_info(scope)
    except (ClientAuthenticationError, DecodeError, KeyError, OSError, ValueError) as error:
        return {"kind": "error", "category": category(error)}
    return {
        "kind": "token",
        "token": token.token,
        "expires_on": token.expires_on,
        "token_type": token.token_type,
        "refresh_on": token.refresh_on,
        "bearer": litellm_bearer(credential, scope),
    }


def acquire_token(step: TokenStep) -> dict[str, Json]:
    transport = FixtureTransport(step.get("replies") or [])
    scope = f"{step['resource']}/.default"
    service_fabric_client = mock.patch.object(
        msal_managed_identity, "_create_service_fabric_http_client", FixtureFactory(transport)
    )
    with fixture_host(step), service_fabric_client:
        outcome = credential_token(step, transport, scope)
    return {"requests": list(transport.requests), "outcome": outcome}


def select_step(step: SelectStep) -> dict[str, Json]:
    with fixture_host(step):
        source = msal_managed_identity.get_managed_identity_source()
    return {"selected": source_name(source)}


def run(argument: Input) -> object:
    steps = [
        select_step(step) if step["action"] == "select" else acquire_token(step)
        for step in argument["steps"]
    ]
    return {"steps": steps}
