import builtins
import io
import json
import os
import time
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from importlib import import_module
from types import SimpleNamespace
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast
from unittest import mock
from urllib.parse import parse_qsl, urlsplit, urlunsplit

import azure.identity
from azure.core.exceptions import ClientAuthenticationError, DecodeError
from azure.identity import CredentialUnavailableError, ManagedIdentityCredential
from azure.identity._internal import within_credential_chain
from azure.identity._internal.msal_managed_identity_client import MsalManagedIdentityClient
from litellm.secret_managers.get_azure_ad_token_provider import get_azure_ad_token_provider
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

NOW = 1_700_000_000
DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
REQUEST_ID = "00000000-0000-4000-8000-000000000001"
KEPT_HEADERS = frozenset(
    (
        "metadata",
        "secret",
        "x-identity-header",
        "x-client-sku",
        "x-client-ver",
        "x-ms-client-request-id",
    )
)
CREDENTIAL_NAMES = {
    "ServiceFabricCredential": "service_fabric",
    "AppServiceCredential": "app_service",
    "AzureMLCredential": "azure_ml",
    "AzureArcCredential": "arc",
    "CloudShellCredential": "cloud_shell",
    "WorkloadIdentityCredential": "workload",
    "ImdsCredential": "imds",
}


class MsalModule(Protocol):
    SERVICE_FABRIC: object
    APP_SERVICE: object
    AZURE_ARC: object
    CLOUD_SHELL: object
    MACHINE_LEARNING: object

    def get_managed_identity_source(self) -> object: ...


msal_managed_identity = cast(MsalModule, import_module("msal.managed_identity"))
core_universal = import_module("azure.core.pipeline.policies._universal")


class Reply(TypedDict):
    status: int
    headers: NotRequired[dict[str, str]]
    body: NotRequired[str]


class HostStep(TypedDict, total=False):
    environment: dict[str, str]
    client_id: str | None
    exclude_workload_identity: bool
    files: dict[str, str]


class SelectStep(HostStep):
    action: Literal["select"]


class TokenStep(HostStep):
    action: Literal["token"]
    scope: NotRequired[str]
    now: NotRequired[int]
    probe: NotRequired[bool | None]
    within_credential_chain: NotRequired[bool]
    replies: NotRequired[list[Reply]]


Step: TypeAlias = SelectStep | TokenStep


class Input(TypedDict):
    steps: list[Step]


class HttpRequestLike(Protocol):
    method: str
    url: str
    headers: Mapping[str, str]
    body: object


class AccessTokenLike(Protocol):
    token: str
    expires_on: int
    token_type: str
    refresh_on: int | None


class FixtureResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.headers = dict(reply.get("headers") or {})
        self.reason = ""
        self.content_type = next(
            (value for name, value in self.headers.items() if name.lower() == "content-type"), None
        )
        self._body = (reply.get("body") or "").encode()

    def text(self, encoding: str | None = None) -> str:
        return self._body.decode("utf-8-sig" if encoding in (None, "utf-8") else encoding)

    def read(self) -> bytes:
        return self._body


class FixtureTransport:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[dict[str, Json]] = []
        self.sleeps: list[float] = []

    def send(self, request: HttpRequestLike, **options: Json) -> FixtureResponse:
        self.requests.append(recorded_request(request, options))
        if not self.replies:
            raise OSError("no reply queued")
        return FixtureResponse(self.replies.pop(0))

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)


class FixtureFiles:
    def __init__(self, files: Mapping[str, str]) -> None:
        self.files = dict(files)

    def open(self, path: str, *_args: object, **_kwargs: object) -> io.StringIO:
        if path not in self.files:
            raise FileNotFoundError(path)
        return io.StringIO(self.files[path])

    def exists(self, path: str) -> bool:
        return path in self.files


def mapping_of(value: object) -> dict[str, str] | None:
    if not isinstance(value, Mapping):
        return None
    pairs = cast("Mapping[object, object]", value).items()
    return {str(name): str(item) for name, item in pairs}


def recorded_request(request: HttpRequestLike, options: Mapping[str, Json]) -> dict[str, Json]:
    parts = urlsplit(request.url)
    timeout = options.get("connection_timeout")
    return cast(
        "dict[str, Json]",
        {
            "method": request.method,
            "url": urlunsplit((parts.scheme, parts.netloc, parts.path, "", "")),
            "params": dict(parse_qsl(parts.query, keep_blank_values=True)),
            "headers": {
                name.lower(): value
                for name, value in request.headers.items()
                if name.lower() in KEPT_HEADERS
            },
            "form": mapping_of(request.body),
            "timeout": timeout
            if isinstance(timeout, int) and not isinstance(timeout, bool)
            else None,
        },
    )


@contextmanager
def fixture_host(step: HostStep, now: int | None = None) -> Generator[None]:
    environment = step.get("environment") or {}
    files = FixtureFiles(step.get("files") or {})
    msal_os = SimpleNamespace(
        environ=os.environ,
        path=SimpleNamespace(exists=files.exists),
        getenv=os.environ.get,
    )
    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch.object(time, "time", return_value=float(now if now is not None else NOW)),
        mock.patch.object(msal_managed_identity, "os", msal_os),
        mock.patch.object(
            msal_managed_identity, "uuid", mock.Mock(uuid4=mock.Mock(return_value=REQUEST_ID))
        ),
        mock.patch.object(
            core_universal, "uuid", mock.Mock(uuid1=mock.Mock(return_value=REQUEST_ID))
        ),
        mock.patch.object(builtins, "open", files.open),
    ):
        yield


@contextmanager
def chain_scope(*, enabled: bool) -> Generator[None]:
    marker = within_credential_chain.set(enabled)
    try:
        yield
    finally:
        within_credential_chain.reset(marker)


def category(error: Exception) -> str:
    if isinstance(error, CredentialUnavailableError):
        return "unavailable"
    if isinstance(error, ClientAuthenticationError):
        return "client_authentication"
    if isinstance(error, (DecodeError, json.JSONDecodeError)):
        return "invalid_json"
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


def litellm_bearer(credential: ManagedIdentityCredential, scope: str, token: str) -> str | None:
    provider = mock.Mock(return_value=token)
    with (
        mock.patch.object(
            azure.identity, "ManagedIdentityCredential", mock.Mock(return_value=credential)
        ),
        mock.patch.object(
            azure.identity, "get_bearer_token_provider", mock.Mock(return_value=provider)
        ),
    ):
        try:
            return get_azure_ad_token_provider(
                azure_scope=scope,
                azure_credential=AzureCredentialType.DeploymentIdentityCredential,
            )()
        except (ClientAuthenticationError, ValueError):
            return None


def select_step(step: SelectStep) -> dict[str, Json]:
    with fixture_host(step):
        try:
            credential = ManagedIdentityCredential(
                client_id=step.get("client_id"),
                transport=FixtureTransport([]),
                _exclude_workload_identity_credential=bool(step.get("exclude_workload_identity")),
            )
            selected = (
                "none"
                if credential._credential is None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                else CREDENTIAL_NAMES[type(credential._credential).__name__]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            )
            inner = (
                source_name(msal_managed_identity.get_managed_identity_source())
                if isinstance(credential._credential, MsalManagedIdentityClient)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                else None
            )
        except Exception as error:  # noqa: BLE001 - the wire contract names every helper exception
            return {"kind": "error", "category": category(error)}
    return {"kind": "selection", "selected": selected, "inner": inner}


def token_step(step: TokenStep) -> dict[str, Json]:
    transport = FixtureTransport(step.get("replies") or [])
    scope = step.get("scope") or DEFAULT_SCOPE
    probe = step.get("probe")
    chain = step.get("within_credential_chain", False)
    bearer: str | None = None
    with fixture_host(step, step.get("now")), chain_scope(
        enabled=probe if probe is not None else chain
    ):
        try:
            credential = ManagedIdentityCredential(
                client_id=step.get("client_id"),
                transport=transport,
                _exclude_workload_identity_credential=bool(step.get("exclude_workload_identity")),
            )
            selected = (
                "none"
                if credential._credential is None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                else CREDENTIAL_NAMES[type(credential._credential).__name__]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            )
            info = cast("AccessTokenLike", credential.get_token_info(scope))
            bearer = litellm_bearer(credential, scope, info.token)
            if selected == "workload":
                outcome: dict[str, Json] = {"kind": "token", "token": info.token}
            else:
                outcome = {
                    "kind": "token",
                    "token": info.token,
                    "expires_on": info.expires_on,
                    "token_type": info.token_type,
                    "refresh_on": info.refresh_on,
                }
        except Exception as error:  # noqa: BLE001 - the wire contract names every helper exception
            outcome = {"kind": "error", "category": category(error)}
            bearer = None
    return {
        **outcome,
        "bearer": bearer,
        "requests": cast("Json", transport.requests),
        "sleeps": cast("Json", transport.sleeps),
    }


def run(argument: Input) -> Json:
    steps = [
        select_step(step) if step["action"] == "select" else token_step(step)
        for step in argument["steps"]
    ]
    return {"steps": cast("Json", steps)}
