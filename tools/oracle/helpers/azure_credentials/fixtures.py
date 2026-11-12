import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.azure import common_utils
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenProviderScript(TypedDict):
    outcome: Literal["token"]
    token: str


class NonTextProviderScript(TypedDict):
    outcome: Literal["non_text"]
    type_name: str


class RaiseProviderScript(TypedDict):
    outcome: Literal["raise"]
    class_name: str
    message: str


ProviderScript: TypeAlias = TokenProviderScript | NonTextProviderScript | RaiseProviderScript


class TokenSourceScript(TypedDict):
    outcome: Literal["token"]
    token: str


class RaiseSourceScript(TypedDict):
    outcome: Literal["raise"]
    class_name: str
    message: str


SourceScript: TypeAlias = TokenSourceScript | RaiseSourceScript


class Params(TypedDict, total=False):
    azure_ad_token: str
    tenant_id: str
    client_id: str
    client_secret: str
    azure_scope: str
    azure_username: str
    azure_password: str


class TokenStep(TypedDict):
    cache: list[str]
    environment: NotRequired[dict[str, str]]
    params: NotRequired[Params]
    refresh: NotRequired[bool]
    provider: NotRequired[ProviderScript]
    sources: NotRequired[dict[str, SourceScript]]


class ClientParams(TypedDict, total=False):
    azure_ad_token: str
    api_key: str


class HeaderStep(TypedDict, total=False):
    headers: dict[str, str]
    client: ClientParams
    provider: ProviderScript


class ResponseStep(TypedDict):
    source: Literal["dict", "httpx"]
    headers: dict[str, str]


class TokenBatch(TypedDict):
    steps: list[TokenStep]


class HeaderBatch(TypedDict):
    steps: list[HeaderStep]


class ResponseBatch(TypedDict):
    steps: list[ResponseStep]


class Input(TypedDict, total=False):
    token: TokenBatch
    request_headers: HeaderBatch
    redact_headers: HeaderBatch
    response_headers: ResponseBatch


@dataclass
class Capture:
    calls: list[dict[str, Json]] = field(default_factory=list)
    provider_calls: int = 0
    sources: dict[str, SourceScript] = field(default_factory=dict)
    held: list[Callable[[], str]] = field(default_factory=list)
    slots: dict[int, int] = field(default_factory=dict)


STATE = Capture()


class FixedProvider:
    def __init__(self, token: str) -> None:
        self.token = token

    def __call__(self) -> str:
        STATE.provider_calls += 1
        return self.token


class NonTextProvider:
    def __init__(self, type_name: str) -> None:
        self.type_name = type_name

    def __call__(self) -> str:
        STATE.provider_calls += 1
        return cast(str, None)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L399


class RaisingProvider:
    def __init__(self, failure: Exception) -> None:
        self.failure = failure

    def __call__(self) -> str:
        STATE.provider_calls += 1
        raise self.failure


def exception_for(script: RaiseProviderScript | RaiseSourceScript) -> Exception:
    class_name = script["class_name"]
    message = script["message"]
    match class_name:
        case "ValueError":
            return ValueError(message)
        case "KeyError":
            return KeyError(message)
        case "TypeError":
            return TypeError(message)
        case _:
            return RuntimeError(message)


def provider_from(script: ProviderScript | None) -> Callable[[], str]:
    if script is None:
        return FixedProvider("provider-token")
    if script["outcome"] == "token":
        return FixedProvider(script["token"])
    if script["outcome"] == "non_text":
        return NonTextProvider(script["type_name"])
    return RaisingProvider(exception_for(script))


def source_provider(script: SourceScript | None, default_token: str) -> Callable[[], str]:
    if script is None:
        return FixedProvider(default_token)
    if script["outcome"] == "token":
        return FixedProvider(script["token"])
    raise exception_for(script)


DEFAULT_SCOPE = "https://cognitiveservices.azure.com/.default"


def entra_id_source(
    tenant_id: str,
    client_id: str,
    client_secret: str,
    scope: str = DEFAULT_SCOPE,
) -> Callable[[], str]:
    STATE.calls.append({
        "source": "client_secret",
        "tenant_id": tenant_id,
        "client_id": client_id,
        "client_secret": client_secret,
        "scope": scope,
    })
    return source_provider(STATE.sources.get("client_secret"), "client-secret-token")


def username_source(
    client_id: str,
    azure_username: str,
    azure_password: str,
    scope: str = DEFAULT_SCOPE,
) -> Callable[[], str]:
    call: dict[str, Json] = {}
    call["source"] = "username_password"
    call["client_id"] = client_id
    call["username"] = azure_username
    call["password"] = azure_password
    call["scope"] = scope
    STATE.calls.append(call)
    return source_provider(STATE.sources.get("username_password"), "username-token")


def sdk_provider(
    azure_scope: str | None = None,
    azure_credential: AzureCredentialType | None = None,
) -> Callable[[], str]:
    if azure_credential is None:
        STATE.calls.append({"source": "sdk_refresh", "scope": azure_scope})
        return source_provider(STATE.sources.get("sdk_refresh"), "refresh-token")
    STATE.calls.append({"source": "default_credential", "scope": azure_scope})
    return source_provider(STATE.sources.get("default_credential"), "default-token")


def oidc_source(
    azure_ad_token: str,
    azure_client_id: str | None = None,
    azure_tenant_id: str | None = None,
    scope: str | None = None,
) -> str:
    STATE.calls.append({
        "source": "oidc",
        "token": azure_ad_token,
        "client_id": azure_client_id,
        "tenant_id": azure_tenant_id,
        "scope": scope,
    })
    script = STATE.sources.get("oidc")
    if script is None:
        return "oidc-token"
    if script["outcome"] == "token":
        return script["token"]
    raise exception_for(script)


@contextmanager
def session(
    environment: dict[str, str], sources: dict[str, SourceScript]
) -> Generator[None]:
    STATE.calls.clear()
    STATE.provider_calls = 0
    STATE.sources.clear()
    STATE.sources.update(sources)
    saved_environment = dict(os.environ)
    saved_refresh = litellm.enable_azure_ad_token_refresh
    saved = (
        common_utils.get_azure_ad_token_from_entra_id,
        common_utils.get_azure_ad_token_from_username_password,
        common_utils.get_azure_ad_token_provider,  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L19
        common_utils.get_azure_ad_token_from_oidc,
    )
    os.environ.clear()
    os.environ.update(environment)
    common_utils.get_azure_ad_token_from_entra_id = entra_id_source
    common_utils.get_azure_ad_token_from_username_password = username_source
    common_utils.get_azure_ad_token_provider = sdk_provider  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L19
    common_utils.get_azure_ad_token_from_oidc = oidc_source
    try:
        yield
    finally:
        (
            common_utils.get_azure_ad_token_from_entra_id,
            common_utils.get_azure_ad_token_from_username_password,
            common_utils.get_azure_ad_token_provider,  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L19
            common_utils.get_azure_ad_token_from_oidc,
        ) = saved
        litellm.enable_azure_ad_token_refresh = saved_refresh
        os.environ.clear()
        os.environ.update(saved_environment)
