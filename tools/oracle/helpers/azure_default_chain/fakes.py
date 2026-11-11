import logging
import os
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Generic, Literal, NotRequired, TypeAlias, TypedDict, TypeVar

from azure.core.credentials import AccessToken, AccessTokenInfo
from azure.core.exceptions import ClientAuthenticationError
from azure.identity import CredentialUnavailableError
from azure.identity._credentials.workload_identity import WORKLOAD_CONFIG_ERROR
from azure.identity._internal import within_credential_chain, within_dac
from litellm._logging import verbose_logger
from typing_extensions import override

NOW = 1_700_000_000

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenScript(TypedDict):
    credential: str
    outcome: Literal["token"]
    token: str


class UnavailableScript(TypedDict):
    credential: str
    outcome: Literal["unavailable"]
    message: str


class DeniedScript(TypedDict):
    credential: str
    outcome: Literal["authentication"]
    message: str


ScriptStep: TypeAlias = TokenScript | UnavailableScript | DeniedScript


class SessionFields(TypedDict, total=False):
    environment: dict[str, str]
    scripts: list[ScriptStep]
    construct_errors: dict[str, str]


class CredentialStep(SessionFields):
    credential: str | None
    scope: str | None


class BoundaryStep(SessionFields):
    scope: str
    invoke: NotRequired[bool]


Step = TypeVar("Step")


class Batch(TypedDict, Generic[Step]):
    steps: list[Step]


class Input(TypedDict, total=False):
    infer: Batch[SessionFields]
    provider: Batch[CredentialStep]
    chain: Batch[CredentialStep]
    try_boundary: Batch[BoundaryStep]


@dataclass
class Capture:
    records: list[dict[str, Json]] = field(default_factory=list)
    calls: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    litellm: list[str] = field(default_factory=list)
    azure: list[str] = field(default_factory=list)
    scripts: dict[str, ScriptStep] = field(default_factory=dict)
    errors: dict[str, str] = field(default_factory=dict)


STATE = Capture()


class LineHandler(logging.Handler):
    def __init__(self, lines: list[str], level: int) -> None:
        super().__init__(level=level)
        self.lines = lines

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append(record.getMessage())


@contextmanager
def capture_logs() -> Generator[None]:
    azure_logger = logging.getLogger("azure.identity")
    azure_handler = LineHandler(STATE.azure, logging.INFO)
    litellm_handler = LineHandler(STATE.litellm, logging.DEBUG)
    azure_level = azure_logger.level
    litellm_level = verbose_logger.level
    litellm_handlers = list(verbose_logger.handlers)
    azure_logger.setLevel(logging.INFO)
    azure_logger.addHandler(azure_handler)
    verbose_logger.handlers.clear()
    verbose_logger.setLevel(logging.DEBUG)
    verbose_logger.addHandler(litellm_handler)
    try:
        yield
    finally:
        verbose_logger.handlers.clear()
        verbose_logger.handlers.extend(litellm_handlers)
        verbose_logger.setLevel(litellm_level)
        azure_logger.removeHandler(azure_handler)
        azure_logger.setLevel(azure_level)


class FakeCredential:
    def __init__(self, key: str, **fields: Json) -> None:
        name = type(self).__name__
        if name in STATE.errors:
            raise ValueError(STATE.errors[name])
        STATE.records.append({**fields, "credential": name})
        self.key = key

    def get_token_info(self, *_scopes: str, **_kwargs: Json) -> AccessTokenInfo:
        STATE.calls.append(self.key)
        if self.key not in STATE.scripts:
            return AccessTokenInfo(f"{self.key}-token", NOW)
        script = STATE.scripts[self.key]
        if script["outcome"] == "token":
            return AccessTokenInfo(script["token"], NOW)
        if script["outcome"] == "unavailable":
            raise CredentialUnavailableError(script["message"])
        raise ClientAuthenticationError(script["message"])

    def get_token(self, *scopes: str, **kwargs: Json) -> AccessToken:
        info = self.get_token_info(*scopes, **kwargs)
        return AccessToken(info.token, info.expires_on)


class ClientSecretCredential(FakeCredential):
    def __init__(self, tenant_id: str, client_id: str, client_secret: str, **_kwargs: Json) -> None:
        super().__init__(
            "ClientSecretCredential",
            tenant_id=tenant_id,
            client_id=client_id,
            client_secret=client_secret,
        )


class CertificateCredential(FakeCredential):
    def __init__(
        self,
        tenant_id: str | None = None,
        client_id: str | None = None,
        certificate_path: str | None = None,
        password: str | None = None,
        *,
        send_certificate_chain: bool = False,
        **_kwargs: Json,
    ) -> None:
        super().__init__(
            "CertificateCredential",
            tenant_id=tenant_id,
            client_id=client_id,
            certificate_path=certificate_path,
            password=password,
            send_certificate_chain=send_certificate_chain,
        )


class UsernamePasswordCredential(FakeCredential):
    def __init__(
        self,
        client_id: str,
        username: str,
        password: str,
        *,
        tenant_id: str | None = None,
        **_kw: Json,
    ) -> None:
        login: dict[str, Json] = {"username": username, "password": password}
        super().__init__(
            "UsernamePasswordCredential", tenant_id=tenant_id, client_id=client_id, **login
        )


class ManagedIdentityCredential(FakeCredential):
    def __init__(self, client_id: str | None = None, **flags: bool | None) -> None:
        fields: dict[str, Json] = {
            "client_id": client_id,
            "exclude_workload_identity": flags.get("_exclude_workload_identity_credential") is True,
            "imds_probe": flags.get("_enable_imds_probe"),
            "within_credential_chain": within_credential_chain.get(),
            "within_default_credential": within_dac.get(),
        }
        kind = "user" if client_id else "system"
        super().__init__(f"ManagedIdentityCredential:{kind}", **fields)


class WorkloadIdentityCredential(FakeCredential):
    def __init__(
        self,
        *,
        tenant_id: str | None = None,
        client_id: str | None = None,
        token_file_path: str | None = None,
        **_kwargs: Json,
    ) -> None:
        arguments = {
            "tenant_id": tenant_id or os.environ.get("AZURE_TENANT_ID"),
            "client_id": client_id or os.environ.get("AZURE_CLIENT_ID"),
            "token_file_path": token_file_path or os.environ.get("AZURE_FEDERATED_TOKEN_FILE"),
        }
        missing = [f"'{name}'" for name, value in arguments.items() if not value]
        if missing:
            raise ValueError(
                f"{WORKLOAD_CONFIG_ERROR}. Missing required arguments: {', '.join(missing)}."
            )
        fields: dict[str, Json] = {
            "tenant_id": arguments["tenant_id"],
            "client_id": arguments["client_id"],
            "token_file": arguments["token_file_path"],
        }
        super().__init__("WorkloadIdentityCredential", **fields)
