import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.azure import common_utils
from litellm.types.secret_managers.get_azure_ad_token_provider import AzureCredentialType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenScript(TypedDict):
    outcome: Literal["token"]
    token: str


class NonTextScript(TypedDict):
    outcome: Literal["non_text"]
    type_name: str


class RaiseScript(TypedDict):
    outcome: Literal["raise"]
    class_name: str
    message: str


Script: TypeAlias = TokenScript | NonTextScript | RaiseScript


class TokenStep(TypedDict):
    cache: list[str]
    environment: NotRequired[dict[str, str]]
    params: NotRequired[dict[str, str]]
    refresh: NotRequired[bool]
    provider: NotRequired[Script]
    sources: NotRequired[dict[str, Script]]


class HeaderStep(TypedDict, total=False):
    headers: dict[str, str]
    client: dict[str, str]
    provider: Script


class ResponseStep(TypedDict):
    source: Literal["dict", "httpx"]
    headers: dict[str, str]


class Input(TypedDict, total=False):
    token: list[TokenStep]
    request_headers: list[HeaderStep]
    redact_headers: list[HeaderStep]
    response_headers: list[ResponseStep]


@dataclass
class Capture:
    calls: list[dict[str, Json]] = field(default_factory=list)
    provider_calls: int = 0
    sources: dict[str, Script] = field(default_factory=dict)
    held: list[Callable[[], str]] = field(default_factory=list)
    slots: dict[int, int] = field(default_factory=dict)


STATE = Capture()


class ScriptedProvider:
    def __init__(self, script: Script) -> None:
        self.script = script

    def __call__(self) -> str:
        STATE.provider_calls += 1
        if self.script["outcome"] == "token":
            return self.script["token"]
        if self.script["outcome"] == "non_text":
            return cast(str, None)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L399
        raise exception_for(self.script)


def exception_for(script: RaiseScript) -> Exception:
    message = script["message"]
    match script["class_name"]:
        case "ValueError":
            return ValueError(message)
        case "KeyError":
            return KeyError(message)
        case "TypeError":
            return TypeError(message)
        case _:
            return RuntimeError(message)


def source_provider(name: str, default_token: str) -> Callable[[], str]:
    script = STATE.sources.get(name)
    if script is None:
        return ScriptedProvider({"outcome": "token", "token": default_token})
    if script["outcome"] == "raise":
        raise exception_for(script)
    return ScriptedProvider(script)


def entra_id_source(**call: str | None) -> Callable[[], str]:
    STATE.calls.append({"source": "client_secret", **call})
    return source_provider("client_secret", "client-secret-token")


def username_source(**call: str | None) -> Callable[[], str]:
    STATE.calls.append({"source": "username_password", **call})
    return source_provider("username_password", "username-token")


def oidc_source(**call: str | None) -> str:
    STATE.calls.append({"source": "oidc", **call})
    return source_provider("oidc", "oidc-token")()


def sdk_provider(
    azure_scope: str | None = None,
    azure_credential: AzureCredentialType | None = None,
) -> Callable[[], str]:
    if azure_credential is None:
        STATE.calls.append({"source": "sdk_refresh", "scope": azure_scope})
        return source_provider("sdk_refresh", "refresh-token")
    STATE.calls.append({"source": "default_credential", "scope": azure_scope})
    return source_provider("default_credential", "default-token")


@contextmanager
def session(
    environment: dict[str, str], sources: dict[str, Script], *, refresh: bool
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
    litellm.enable_azure_ad_token_refresh = refresh
    common_utils.get_azure_ad_token_from_entra_id = entra_id_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L344
    common_utils.get_azure_ad_token_from_username_password = username_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L354
    common_utils.get_azure_ad_token_provider = sdk_provider  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L19
    common_utils.get_azure_ad_token_from_oidc = oidc_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L364
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
