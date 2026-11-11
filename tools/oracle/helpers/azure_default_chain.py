import logging
import os
import warnings
from collections.abc import Generator
from contextlib import contextmanager
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

from azure.core.credentials import AccessToken, AccessTokenInfo
from azure.core.exceptions import ClientAuthenticationError
from azure.identity import CredentialUnavailableError
from azure.identity._credentials.default import DefaultAzureCredential as RealDefaultAzureCredential
from azure.identity._credentials.workload_identity import WORKLOAD_CONFIG_ERROR
from azure.identity._internal import within_credential_chain, within_dac
from litellm._logging import verbose_logger
from litellm.llms.azure.common_utils import BaseAzureLLM
from litellm.secret_managers.get_azure_ad_token_provider import (
    get_azure_ad_token_provider,
    infer_credential_type_from_environment,
)
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

NOW = 1_700_000_000
DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
SELECTION_PREFIX = "For Azure AD Token Provider"
ATTEMPT_PREFIX = "Attempting to use DefaultAzureCredential"
SUCCESS_PREFIX = "Successfully obtained Azure AD token provider"
FAILED_PREFIX = "DefaultAzureCredential failed: "
MISSING_PASSWORD_ARGUMENTS = ["client_id", "username", "password"]


class ScriptStep(TypedDict):
    credential: str
    outcome: Literal["token", "unavailable", "authentication"]
    token: NotRequired[str]
    message: NotRequired[str]


class StepFields(TypedDict, total=False):
    environment: dict[str, str]
    scripts: list[ScriptStep]
    construct_errors: dict[str, str]


class InferStep(StepFields):
    pass


class ProviderStep(StepFields):
    credential: str | None
    scope: str | None


class ChainStep(ProviderStep):
    pass


class TryStep(StepFields):
    scope: str
    invoke: NotRequired[bool]


class InferInput(TypedDict):
    steps: list[InferStep]


class ProviderInput(TypedDict):
    steps: list[ProviderStep]


class ChainInput(TypedDict):
    steps: list[ChainStep]


class TryInput(TypedDict):
    steps: list[TryStep]


class Input(TypedDict, total=False):
    infer: NotRequired[InferInput]
    provider: NotRequired[ProviderInput]
    chain: NotRequired[ChainInput]
    try_boundary: NotRequired[TryInput]


RECORDS: list[dict[str, Json]] = []
CALLS: list[str] = []
WARNINGS: list[str] = []
LITELLM_LINES: list[str] = []
AZURE_LINES: list[str] = []
SCRIPTS: dict[str, ScriptStep] = {}
CONSTRUCT_ERRORS: dict[str, str] = {}


def fail_construction(name: str) -> None:
    message = CONSTRUCT_ERRORS.get(name)
    if message is not None:
        raise ValueError(message)


class Script:
    def __init__(self, credential: str) -> None:
        self.credential = credential
        self.step = SCRIPTS.get(credential)

    def token(self) -> AccessToken:
        CALLS.append(self.credential)
        step = self.step
        if step is None:
            return AccessToken(f"{self.credential}-token", NOW)
        outcome = step["outcome"]
        if outcome == "token":
            return AccessToken(step.get("token", ""), NOW)
        message = step.get("message", "")
        if outcome == "unavailable":
            raise CredentialUnavailableError(message)
        raise ClientAuthenticationError(message)


class ClientSecretCredential:
    def __init__(
        self, client_id: str, client_secret: str, tenant_id: str, **_kwargs: object
    ) -> None:
        fail_construction("ClientSecretCredential")
        RECORDS.append(
            {
                "tenant_id": tenant_id,
                "client_id": client_id,
                "client_secret": client_secret,
                "credential": "ClientSecretCredential",
            }
        )
        self.script = Script("ClientSecretCredential")

    def get_token(self, *_scopes: str, **_kwargs: object) -> AccessToken:
        return self.script.token()

    def get_token_info(self, *_scopes: str, **_kwargs: object) -> AccessTokenInfo:
        token = self.script.token()
        return AccessTokenInfo(token.token, token.expires_on)


class CertificateCredential:
    def __init__(
        self,
        tenant_id: str | None = None,
        client_id: str | None = None,
        certificate_path: str | None = None,
        password: str | None = None,
        *,
        send_certificate_chain: bool = False,
        **_kwargs: object,
    ) -> None:
        fail_construction("CertificateCredential")
        RECORDS.append(
            {
                "tenant_id": tenant_id,
                "client_id": client_id,
                "certificate_path": certificate_path,
                "password": password,
                "send_certificate_chain": send_certificate_chain,
                "credential": "CertificateCredential",
            }
        )
        self.script = Script("CertificateCredential")

    def get_token(self, *_scopes: str, **_kwargs: object) -> AccessToken:
        return self.script.token()

    def get_token_info(self, *_scopes: str, **_kwargs: object) -> AccessTokenInfo:
        token = self.script.token()
        return AccessTokenInfo(token.token, token.expires_on)


class UsernamePasswordCredential:
    def __init__(
        self,
        client_id: str | None = None,
        username: str | None = None,
        password: str | None = None,
        **_kwargs: object,
    ) -> None:
        fail_construction("UsernamePasswordCredential")
        RECORDS.append(
            {
                "tenant_id": cast("Json", _kwargs.get("tenant_id")),
                "client_id": client_id,
                "username": username,
                "password": password,
                "credential": "UsernamePasswordCredential",
            }
        )
        self.script = Script("UsernamePasswordCredential")

    def get_token(self, *_scopes: str, **_kwargs: object) -> AccessToken:
        return self.script.token()

    def get_token_info(self, *_scopes: str, **_kwargs: object) -> AccessTokenInfo:
        token = self.script.token()
        return AccessTokenInfo(token.token, token.expires_on)


class ManagedIdentityCredential:
    def __init__(self, client_id: str | None = None, **_kwargs: object) -> None:
        fail_construction("ManagedIdentityCredential")
        exclude = cast("bool", _kwargs.pop("_exclude_workload_identity_credential", False))
        probe = cast("bool | None", _kwargs.pop("_enable_imds_probe", None))
        script_name = (
            "ManagedIdentityCredential:user" if client_id else "ManagedIdentityCredential:system"
        )
        RECORDS.append(
            {
                "client_id": client_id,
                "exclude_workload_identity": exclude,
                "imds_probe": probe,
                "within_credential_chain": within_credential_chain.get(),
                "within_default_credential": within_dac.get(),
                "credential": "ManagedIdentityCredential",
            }
        )
        self.script = Script(script_name)

    def get_token(self, *_scopes: str, **_kwargs: object) -> AccessToken:
        return self.script.token()

    def get_token_info(self, *_scopes: str, **_kwargs: object) -> AccessTokenInfo:
        token = self.script.token()
        return AccessTokenInfo(token.token, token.expires_on)


class WorkloadIdentityCredential:
    def __init__(
        self,
        *,
        tenant_id: str | None = None,
        client_id: str | None = None,
        token_file_path: str | None = None,
        **_kwargs: object,
    ) -> None:
        fail_construction("WorkloadIdentityCredential")
        tenant_id = tenant_id or os.environ.get("AZURE_TENANT_ID")
        client_id = client_id or os.environ.get("AZURE_CLIENT_ID")
        token_file_path = token_file_path or os.environ.get("AZURE_FEDERATED_TOKEN_FILE")
        missing: list[str] = []
        if not tenant_id:
            missing.append("'tenant_id'")
        if not client_id:
            missing.append("'client_id'")
        if not token_file_path:
            missing.append("'token_file_path'")
        if missing:
            raise ValueError(
                f"{WORKLOAD_CONFIG_ERROR}. Missing required arguments: {', '.join(missing)}."
            )
        RECORDS.append(
            {
                "tenant_id": tenant_id,
                "client_id": client_id,
                "token_file": token_file_path,
                "credential": "WorkloadIdentityCredential",
            }
        )
        self.script = Script("WorkloadIdentityCredential")

    def get_token(self, *_scopes: str, **_kwargs: object) -> AccessToken:
        return self.script.token()

    def get_token_info(self, *_scopes: str, **_kwargs: object) -> AccessTokenInfo:
        token = self.script.token()
        return AccessTokenInfo(token.token, token.expires_on)


class DefaultAzureCredential(RealDefaultAzureCredential):
    def __init__(self, **kwargs: object) -> None:
        kwargs.update(
            {
                "exclude_shared_token_cache_credential": True,
                "exclude_visual_studio_code_credential": True,
                "exclude_cli_credential": True,
                "exclude_developer_cli_credential": True,
                "exclude_powershell_credential": True,
                "exclude_interactive_browser_credential": True,
                "exclude_broker_credential": True,
            }
        )
        super().__init__(**kwargs)


class AzureCaptureHandler(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.INFO)

    def emit(self, record: logging.LogRecord) -> None:
        AZURE_LINES.append(record.getMessage())


def capture_litellm(message: object, *args: object, **_kwargs: object) -> None:
    if isinstance(message, str):
        LITELLM_LINES.append(message % args if args else message)
    else:
        LITELLM_LINES.append(str(message))


@contextmanager
def session(step: StepFields) -> Generator[None]:
    RECORDS.clear()
    CALLS.clear()
    WARNINGS.clear()
    LITELLM_LINES.clear()
    AZURE_LINES.clear()
    SCRIPTS.clear()
    CONSTRUCT_ERRORS.clear()
    SCRIPTS.update({script["credential"]: script for script in step.get("scripts", [])})
    CONSTRUCT_ERRORS.update(step.get("construct_errors", {}))
    azure_logger = logging.getLogger("azure.identity")
    handler = AzureCaptureHandler()
    previous_level = azure_logger.level
    azure_logger.setLevel(logging.INFO)
    azure_logger.addHandler(handler)
    with (
        mock.patch.dict(os.environ, dict(step.get("environment", {})), clear=True),
        mock.patch("azure.identity.DefaultAzureCredential", DefaultAzureCredential),
        mock.patch("azure.identity.ClientSecretCredential", ClientSecretCredential),
        mock.patch("azure.identity.CertificateCredential", CertificateCredential),
        mock.patch("azure.identity.ManagedIdentityCredential", ManagedIdentityCredential),
        mock.patch(
            "azure.identity._credentials.environment.ClientSecretCredential",
            ClientSecretCredential,
        ),
        mock.patch(
            "azure.identity._credentials.environment.CertificateCredential",
            CertificateCredential,
        ),
        mock.patch(
            "azure.identity._credentials.environment.UsernamePasswordCredential",
            UsernamePasswordCredential,
        ),
        mock.patch(
            "azure.identity._credentials.default.ManagedIdentityCredential",
            ManagedIdentityCredential,
        ),
        mock.patch("azure.identity.WorkloadIdentityCredential", WorkloadIdentityCredential),
        mock.patch(
            "azure.identity._credentials.default.WorkloadIdentityCredential",
            WorkloadIdentityCredential,
        ),
        mock.patch.object(verbose_logger, "info", capture_litellm),
        mock.patch.object(verbose_logger, "debug", capture_litellm),
        mock.patch.object(verbose_logger, "warning", capture_litellm),
        warnings.catch_warnings(record=True) as caught,
    ):
        warnings.simplefilter("always")
        try:
            yield
        finally:
            WARNINGS.extend(str(item.message) for item in caught)
            azure_logger.removeHandler(handler)
            azure_logger.setLevel(previous_level)


def records() -> Json:
    return cast("Json", list(RECORDS))


def selection_log(lines: list[str]) -> str | None:
    return next((line for line in lines if line.startswith(SELECTION_PREFIX)), None)


def success_log(lines: list[str]) -> str | None:
    return next((line for line in lines if " acquired a token from " in line), None)


def warning_log(lines: list[str]) -> str | None:
    return next((line for line in lines if "failed to retrieve a token" in line), None)


def error_value(
    category: str, variable: str | None, message: str | None, missing: list[str]
) -> dict[str, Json]:
    return {
        "category": category,
        "variable": variable,
        "message": message,
        "missing": cast("Json", missing),
    }


def provider_error(error: BaseException) -> dict[str, Json]:
    if isinstance(error, KeyError):
        key = error.args[0] if error.args else ""
        return error_value("missing_environment", str(key), None, [])
    if isinstance(error, TypeError):
        return error_value("type_error", None, None, MISSING_PASSWORD_ARGUMENTS)
    text = str(error)
    if text.startswith(
        (WORKLOAD_CONFIG_ERROR, "Invalid value for AZURE_TOKEN_CREDENTIALS")
    ):
        return error_value("invalid_value", None, text, [])
    return error_value("construction", None, text, [])


def credential_arg(name: str | None) -> AzureCredentialType | str | None:
    if not name:
        return None
    try:
        return AzureCredentialType(name)
    except ValueError:
        # litellm resolves names outside its enum through getattr and the AZURE_CREDENTIAL env
        return name


def litellm_arg(explicit: AzureCredentialType | str | None) -> AzureCredentialType | None:
    if isinstance(explicit, str):
        os.environ["AZURE_CREDENTIAL"] = explicit
        return None
    return explicit


def explicit_name(explicit: AzureCredentialType | str | None) -> str:
    if explicit is None:
        return ""
    return explicit.value if isinstance(explicit, AzureCredentialType) else explicit


def resolved_scope(explicit: str | None) -> str:
    if explicit is not None:
        return explicit
    environment_scope = os.environ.get("AZURE_SCOPE")
    return environment_scope or DEFAULT_SCOPE


def chosen_name(explicit: AzureCredentialType | str | None) -> str:
    name = explicit_name(explicit)
    if name:
        return name
    environment_credential = os.environ.get("AZURE_CREDENTIAL")
    if environment_credential:
        return environment_credential
    return str(infer_credential_type_from_environment().value)


def chain_name(chosen: str) -> str:
    if chosen == "DefaultAzureCredential":
        return "DefaultAzureCredential"
    if chosen == "DeploymentIdentityCredential":
        return "ChainedTokenCredential"
    return chosen


def infer_step(step: InferStep) -> dict[str, Json]:
    with session(step):
        return {"credential": str(infer_credential_type_from_environment().value)}


def provider_step(step: ProviderStep) -> dict[str, Json]:
    with session(step):
        explicit = credential_arg(step.get("credential"))
        try:
            get_azure_ad_token_provider(
                azure_scope=step.get("scope"),
                azure_credential=litellm_arg(explicit),
            )
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            return {
                "kind": "error",
                "error": provider_error(error),
                "constructed": records(),
                "log": selection_log(LITELLM_LINES),
            }
        report = {
            "kind": "credential",
            "chosen": chosen_name(explicit),
            "scope": resolved_scope(step.get("scope")),
            "constructed": records(),
            "log": selection_log(LITELLM_LINES),
        }
    report["warnings"] = cast("Json", list(WARNINGS))
    return report


def chain_step(step: ChainStep) -> dict[str, Json]:
    with session(step):
        explicit = credential_arg(step.get("credential"))
        try:
            provider = get_azure_ad_token_provider(
                azure_scope=step.get("scope"),
                azure_credential=litellm_arg(explicit),
            )
        except Exception as error:  # noqa: BLE001 - the wire contract compares the raised outcome
            return {
                "kind": "error",
                "error": provider_error(error),
                "constructed": records(),
                "log": selection_log(LITELLM_LINES),
            }
        chosen = chosen_name(explicit)
        scope = resolved_scope(step.get("scope"))
        failure = None
        try:
            token = provider()
        except (ClientAuthenticationError, CredentialUnavailableError) as error:
            failure = error.message
            token = ""
    report: dict[str, Json] = {
        "kind": "token",
        "chosen": chosen,
        "scope": scope,
        "chain": chain_name(chosen),
        "constructed": records(),
        "warnings": cast("Json", list(WARNINGS)),
        "log": selection_log(LITELLM_LINES),
        "token_calls": cast("Json", list(CALLS)),
    }
    if failure is not None:
        report["kind"] = "chain_error"
        report["message"] = failure
        report["warning"] = warning_log(AZURE_LINES)
        return report
    report["token"] = token
    report["acquired"] = success_log(AZURE_LINES)
    return report


def boundary_log(line: str) -> bool:
    return line.startswith((ATTEMPT_PREFIX, SUCCESS_PREFIX, FAILED_PREFIX))


def boundary_step(step: TryStep) -> dict[str, Json]:
    with session(step):
        provider = BaseAzureLLM._try_get_default_azure_credential_provider(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            step["scope"]
        )
        logs = [line for line in LITELLM_LINES if boundary_log(line)]
        failure = next((line for line in logs if line.startswith(FAILED_PREFIX)), None)
        provider_present = provider is not None
        result: Json = None
        if provider is not None and step.get("invoke", True):
            try:
                token = provider()
            except ClientAuthenticationError as error:
                result = {"kind": "chain_error", "message": error.message}
            else:
                result = {"kind": "token", "token": token}
    return {
        "kind": "boundary",
        "provider": provider_present,
        "constructed": records(),
        "logs": cast("Json", logs),
        "failure": failure,
        "warnings": cast("Json", list(WARNINGS)),
        "result": result,
    }


def run(argument: Input) -> Json:
    infer = argument.get("infer")
    if infer is not None:
        return cast("Json", {"steps": [infer_step(step) for step in infer["steps"]]})
    provider = argument.get("provider")
    if provider is not None:
        return cast(
            "Json", {"steps": [provider_step(step) for step in provider["steps"]]}
        )
    chain = argument.get("chain")
    if chain is not None:
        return cast("Json", {"steps": [chain_step(step) for step in chain["steps"]]})
    boundary = argument.get("try_boundary")
    if boundary is None:
        raise ValueError("expected one of infer, provider, chain or try_boundary")
    return cast(
        "Json",
        {"steps": [boundary_step(step) for step in boundary["steps"]]},
    )
