import base64
import os
import tempfile
import time
from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from typing import Self, TypeAlias, TypedDict, cast
from unittest import mock

import azure.identity
import jwt
from azure.core.exceptions import ClientAuthenticationError
from azure.identity import CertificateCredential
from litellm.secret_managers.get_azure_ad_token_provider import get_azure_ad_token_provider
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
NOW = 1_700_000_000


class Reply(TypedDict):
    status: int
    body: str


class Step(TypedDict, total=False):
    tenant_id: str
    client_id: str
    certificate: str
    certificate_base64: str
    missing_file: bool
    password: str
    scope: str
    send_certificate_chain: bool
    replies: list[Reply]
    advance: int


class Input(TypedDict):
    steps: list[Step]


# msal asks for the authority aliases after an error reply, which betterlite never does
INSTANCE_METADATA: Reply = {
    "status": 200,
    "body": '{"metadata":[{"aliases":["login.microsoftonline.com"]}]}',
}


class FixtureResponse:
    def __init__(self, reply: Reply) -> None:
        self.status_code = reply["status"]
        self.text = reply["body"]
        self.headers: dict[str, str] = {}

    def raise_for_status(self) -> None:
        if self.status_code >= 500:  # noqa: PLR2004
            raise RuntimeError(f"HTTP Error: {self.status_code}")


class FixtureClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.posts: list[dict[str, Json]] = []
        self.jtis: set[str] = set()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def close(self) -> None:
        return None

    def get_error_response(self, _result: object) -> None:
        return None

    def get(self, url: str, **_kwargs: object) -> FixtureResponse:
        if url.endswith("/discovery/instance"):
            return FixtureResponse(INSTANCE_METADATA)
        return FixtureResponse(self.replies.pop(0))

    def post(
        self,
        url: str,
        data: dict[str, str] | None = None,
        headers: dict[str, str] | None = None,
        **_kwargs: object,
    ) -> FixtureResponse:
        form = {
            name: value.decode() if isinstance(value, bytes) else str(value)
            for name, value in (data or {}).items()
        }
        self.posts.append({"url": url, "form": cast("Json", form), "headers": dict(headers or {})})
        return FixtureResponse(self.replies.pop(0))


@contextmanager
def fixture_sdk(client: FixtureClient) -> Generator[mock.MagicMock]:
    with (
        mock.patch.object(time, "time", return_value=float(NOW)) as clock,
        mock.patch("azure.identity._internal.msal_credentials.MsalClient", return_value=client),
    ):
        yield clock


def build_credential(step: Step, path: str) -> tuple[CertificateCredential, str]:
    tenant_id = step.get("tenant_id", "tenant")
    client_id = step.get("client_id", "client")
    password = step.get("password")
    if step.get("send_certificate_chain"):
        chained = CertificateCredential(
            tenant_id,
            client_id,
            certificate_path=path,
            password=password,
            send_certificate_chain=True,
        )
        return chained, step.get("scope", DEFAULT_SCOPE)
    environment = {
        "AZURE_TENANT_ID": tenant_id,
        "AZURE_CLIENT_ID": client_id,
        "AZURE_CERTIFICATE_PATH": path,
    }
    if password is not None:
        environment["AZURE_CERTIFICATE_PASSWORD"] = password
    built: list[tuple[CertificateCredential, str]] = []

    def capture(credential: CertificateCredential, scope: str) -> object:
        built.append((credential, scope))
        return object()

    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch.object(azure.identity, "get_bearer_token_provider", capture),
    ):
        get_azure_ad_token_provider(step.get("scope"), AzureCredentialType.CertificateCredential)
    return built[0]


def request_report(post: dict[str, Json], client: FixtureClient) -> Json:
    form = cast("dict[str, str]", post["form"])
    assertion = form["client_assertion"]
    claims = cast("dict[str, Json]", jwt.decode(assertion, options={"verify_signature": False}))
    for name in ("exp", "iat"):
        claims[name] = int(cast("float", claims[name]))
    jti = cast("str", claims["jti"])
    claims["jti"] = "repeated" if jti in client.jtis else "new"
    client.jtis.add(jti)
    return {
        "url": post["url"],
        "accept": cast("dict[str, str]", post["headers"]).get("Accept"),
        "form": {name: value for name, value in form.items() if name != "client_assertion"},
        "header": cast("Json", jwt.get_unverified_header(assertion)),
        "claims": cast("Json", claims),
    }


def attempt(credential: CertificateCredential, scope: str, client: FixtureClient) -> Json:
    seen = len(client.posts)
    try:
        info = credential.get_token_info(scope)
    except Exception as error:  # noqa: BLE001 - the report carries the SDK's own message
        return {"error": error_message(error)}
    return {
        "token": info.token,
        "expires_on": info.expires_on,
        "token_type": info.token_type,
        "refresh_on": info.refresh_on,
        "request": request_report(client.posts[-1], client) if len(client.posts) > seen else None,
    }


def error_message(error: Exception) -> str:
    if isinstance(error, OSError):
        return "unreadable"
    if isinstance(error, ClientAuthenticationError):
        return error.message
    return str(error)


def run_step(step: Step) -> Json:
    client = FixtureClient(step.get("replies", []))
    with tempfile.TemporaryDirectory() as folder, fixture_sdk(client) as clock:
        path = Path(folder, "certificate")
        if not step.get("missing_file"):
            text = step.get("certificate")
            encoded = step.get("certificate_base64", "")
            path.write_bytes(text.encode() if text is not None else base64.b64decode(encoded))
        try:
            credential, scope = build_credential(step, str(path))
        except Exception as error:  # noqa: BLE001 - the report carries the SDK's own message
            return [{"error": error_message(error)}]
        attempts = [attempt(credential, scope, client)]
        advance = step.get("advance")
        if advance is not None:
            clock.return_value = float(NOW + advance)
            attempts.append(attempt(credential, scope, client))
    return attempts


def run(argument: Input) -> Json:
    return [run_step(step) for step in argument["steps"]]
