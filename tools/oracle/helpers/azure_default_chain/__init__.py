import os
import warnings
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass

from azure import identity
from azure.core.credentials import TokenCredential
from azure.core.exceptions import ClientAuthenticationError
from azure.identity._credentials import default as default_module
from azure.identity._credentials import environment as environment_module
from azure.identity._credentials.default import DefaultAzureCredential as RealDefault
from azure.identity._credentials.workload_identity import WORKLOAD_CONFIG_ERROR
from litellm.llms.azure.common_utils import BaseAzureLLM
from litellm.secret_managers.get_azure_ad_token_provider import (
    get_azure_ad_token_provider,
    infer_credential_type_from_environment,
)
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

from azure_default_chain import fakes
from azure_default_chain.fakes import (
    STATE,
    BoundaryStep,
    CredentialStep,
    Input,
    Json,
    SessionFields,
    capture_logs,
)

DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"
BOUNDARY_PREFIXES = (
    "Attempting to use DefaultAzureCredential",
    "Successfully obtained Azure AD token provider",
    "DefaultAzureCredential failed: ",
)
SELECTION_MARKER = "For Azure AD Token Provider"
INVALID_FILTER = "Invalid value for AZURE_TOKEN_CREDENTIALS"
PASSWORD_ARGUMENTS: list[Json] = ["client_id", "username", "password"]


class DefaultAzureCredential(RealDefault):
    def __init__(self, **kwargs: Json) -> None:
        super().__init__(
            exclude_shared_token_cache_credential=True,
            exclude_visual_studio_code_credential=True,
            exclude_cli_credential=True,
            exclude_developer_cli_credential=True,
            exclude_powershell_credential=True,
            exclude_interactive_browser_credential=True,
            exclude_broker_credential=True,
            **kwargs,
        )


@dataclass(frozen=True)
class Classes:
    default: type[TokenCredential]
    client_secret: type[TokenCredential]
    certificate: type[TokenCredential]
    username_password: type[TokenCredential]
    managed: type[TokenCredential]
    workload: type[TokenCredential]


REAL = Classes(
    identity.DefaultAzureCredential,  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.ClientSecretCredential,  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.CertificateCredential,  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.UsernamePasswordCredential,  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.ManagedIdentityCredential,  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.WorkloadIdentityCredential,  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
)
FAKE = Classes(
    DefaultAzureCredential,
    fakes.ClientSecretCredential,
    fakes.CertificateCredential,
    fakes.UsernamePasswordCredential,
    fakes.ManagedIdentityCredential,
    fakes.WorkloadIdentityCredential,
)


def install(classes: Classes) -> None:
    identity.DefaultAzureCredential = classes.default  # type: ignore[misc, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.ClientSecretCredential = classes.client_secret  # type: ignore[misc, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.CertificateCredential = classes.certificate  # type: ignore[misc, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.UsernamePasswordCredential = classes.username_password  # type: ignore[misc, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.ManagedIdentityCredential = classes.managed  # type: ignore[misc, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    identity.WorkloadIdentityCredential = classes.workload  # type: ignore[misc, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/__init__.py
    environment_module.ClientSecretCredential = classes.client_secret  # type: ignore[attr-defined, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/_credentials/environment.py
    environment_module.CertificateCredential = classes.certificate  # type: ignore[attr-defined, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/_credentials/environment.py
    environment_module.UsernamePasswordCredential = classes.username_password  # type: ignore[attr-defined, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/_credentials/environment.py
    default_module.ManagedIdentityCredential = classes.managed  # type: ignore[attr-defined, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/_credentials/default.py
    default_module.WorkloadIdentityCredential = classes.workload  # type: ignore[attr-defined, assignment]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/identity/azure-identity/azure/identity/_credentials/default.py


@contextmanager
def session(step: SessionFields) -> Generator[None]:
    STATE.records.clear()
    STATE.calls.clear()
    STATE.warnings.clear()
    STATE.litellm.clear()
    STATE.azure.clear()
    STATE.scripts.clear()
    STATE.errors.clear()
    if "scripts" in step:
        STATE.scripts.update({script["credential"]: script for script in step["scripts"]})
    if "construct_errors" in step:
        STATE.errors.update(step["construct_errors"])
    saved_environment = dict(os.environ)
    os.environ.clear()
    if "environment" in step:
        os.environ.update(step["environment"])
    install(FAKE)
    with capture_logs(), warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        try:
            yield
        finally:
            STATE.warnings.extend(str(item.message) for item in caught)
            install(REAL)
            os.environ.clear()
            os.environ.update(saved_environment)


def first_line(lines: list[str], marker: str) -> str | None:
    for line in lines:
        if marker in line:
            return line
    return None


def selection_log() -> str | None:
    line = first_line(STATE.litellm, SELECTION_MARKER)
    if line is None:
        return None
    return line.replace("AzureCredentialType.", "")


def error_report(category: str, variable: str | None, message: str | None) -> dict[str, Json]:
    return {
        "kind": "error",
        "error": {
            "category": category,
            "variable": variable,
            "message": message,
            "missing": PASSWORD_ARGUMENTS if category == "type_error" else [],
        },
        "constructed": list(STATE.records),
        "log": selection_log(),
    }


def build_provider(step: CredentialStep) -> Callable[[], str] | dict[str, Json]:
    argument = None
    if step["credential"]:
        try:
            argument = AzureCredentialType(step["credential"])
        except ValueError:
            os.environ["AZURE_CREDENTIAL"] = step["credential"]
    try:
        return get_azure_ad_token_provider(azure_scope=step["scope"], azure_credential=argument)
    except KeyError as error:
        return error_report("missing_environment", str(error).strip("'"), None)
    except TypeError:
        return error_report("type_error", None, None)
    except ValueError as error:
        text = str(error)
        if text.startswith((WORKLOAD_CONFIG_ERROR, INVALID_FILTER)):
            return error_report("invalid_value", None, text)
        return error_report("construction", None, text)


def run_provider(provider: Callable[[], str]) -> dict[str, Json]:
    try:
        token = provider()
    except ClientAuthenticationError as error:  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/core/azure-core/azure/core/exceptions.py
        return {
            "kind": "chain_error",
            "message": error.message,
            "warning": first_line(STATE.azure, "failed to retrieve a token"),
        }
    return {
        "kind": "token",
        "token": token,
        "acquired": first_line(STATE.azure, " acquired a token from "),
    }


def credential_step(step: CredentialStep, *, invoke: bool) -> dict[str, Json]:
    with session(step):
        match build_provider(step):
            case dict() as failure:
                return failure
            case provider:
                outcome = run_provider(provider) if invoke else {}
        chosen = (
            step["credential"]
            or os.environ.get("AZURE_CREDENTIAL")
            or infer_credential_type_from_environment().value
        )
        scope = step["scope"]
        if scope is None:
            scope = os.environ.get("AZURE_SCOPE") or DEFAULT_SCOPE
    report: dict[str, Json] = {
        "kind": "credential",
        "chosen": chosen,
        "scope": scope,
        "constructed": list(STATE.records),
        "warnings": list(STATE.warnings),
        "log": selection_log(),
    }
    if invoke:
        report.update(outcome)
        report["chain"] = chosen
        report["token_calls"] = list(STATE.calls)
        if chosen == "DeploymentIdentityCredential":
            report["chain"] = "ChainedTokenCredential"
    return report


def boundary_step(step: BoundaryStep) -> dict[str, Json]:
    result: Json = None
    with session(step):
        provider = BaseAzureLLM._try_get_default_azure_credential_provider(step["scope"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L452
        logs = [line for line in STATE.litellm if line.startswith(BOUNDARY_PREFIXES)]
        if provider is not None and step.get("invoke") is not False:
            try:
                result = {"kind": "token", "token": provider()}
            except ClientAuthenticationError as error:  # type: ignore[misc]  # https://github.com/Azure/azure-sdk-for-python/blob/3bdfb63001d329b807eb201a2e631c86782f0bf6/sdk/core/azure-core/azure/core/exceptions.py
                result = {"kind": "chain_error", "message": error.message}
    return {
        "kind": "boundary",
        "provider": provider is not None,
        "constructed": list(STATE.records),
        "logs": list(logs),
        "failure": next((line for line in logs if line.startswith(BOUNDARY_PREFIXES[2])), None),
        "warnings": list(STATE.warnings),
        "result": result,
    }


def infer_step(step: SessionFields) -> dict[str, Json]:
    with session(step):
        return {"credential": infer_credential_type_from_environment().value}


def run(argument: Input) -> Json:
    steps: list[Json]
    if "infer" in argument:
        steps = [infer_step(step) for step in argument["infer"]["steps"]]
    elif "provider" in argument:
        steps = [credential_step(step, invoke=False) for step in argument["provider"]["steps"]]
    elif "chain" in argument:
        steps = [credential_step(step, invoke=True) for step in argument["chain"]["steps"]]
    elif "try_boundary" in argument:
        steps = [boundary_step(step) for step in argument["try_boundary"]["steps"]]
    else:
        raise ValueError("expected one of infer, provider, chain or try_boundary")
    return {"steps": steps}
