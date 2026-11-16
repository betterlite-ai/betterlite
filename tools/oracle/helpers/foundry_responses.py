import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict
from unittest import mock

import httpx
import litellm
from litellm.llms.azure import common_utils as azure_common
from litellm.llms.azure_ai.decisions.transformation import AzureAIDecisionsConfig
from litellm.llms.azure_ai.responses.transformation import AzureAIResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

DEFAULT_SCOPE: Final[str] = "https://cognitiveservices.azure.com/.default"
MODEL: Final[str] = "Microsoft-Decision-1"
JSON: Final = TypeAdapter(JsonValue)
Headers: TypeAlias = dict[str, str]
Object: TypeAlias = dict[str, JsonValue]


class EntraParams(TypedDict, total=False):
    azure_ad_token: str
    tenant_id: str
    client_id: str
    client_secret: str
    azure_scope: str
    azure_username: str
    azure_password: str


class UrlStep(TypedDict):
    kind: Literal["url", "missing_base"]
    api_base: NotRequired[str]
    global_api_base: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class AuthStep(TypedDict):
    kind: Literal["auth"]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    global_api_base: NotRequired[str]
    global_api_key: NotRequired[str]
    headers: NotRequired[Headers]
    params: NotRequired[EntraParams]
    environment: NotRequired[dict[str, str]]


class DecisionsStep(TypedDict):
    kind: Literal["decisions_route"]
    model: str
    api_base: NotRequired[str]
    api_key: NotRequired[str]


class WebsocketStep(TypedDict):
    kind: Literal["no_native_websocket"]


Step: TypeAlias = UrlStep | AuthStep | DecisionsStep | WebsocketStep


class Input(TypedDict):
    steps: list[Step]


class ResponsesConfig(Protocol):
    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def supports_native_websocket(self) -> bool: ...

    def validate_environment(
        self, headers: Headers, model: str, litellm_params: GenericLiteLLMParams | None
    ) -> dict[str, str]: ...

    @property
    def custom_llm_provider(self) -> LlmProviders: ...


RESPONSES: ResponsesConfig = AzureAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/responses/transformation.py#L27-L28


class ScriptedToken:
    def __init__(self, token: str) -> None:
        self.token = token

    def __call__(self) -> str:
        return self.token


def client_secret_grant(
    tenant_id: str,
    client_id: str,
    client_secret: str,
    scope: str = DEFAULT_SCOPE,
) -> Callable[[], str]:
    return ScriptedToken(
        f"client_secret tenant={tenant_id} client={client_id} secret={client_secret} scope={scope}"
    )


@contextmanager
def grant_overrides() -> Generator[None]:
    saved = azure_common.get_azure_ad_token_from_entra_id
    azure_common.get_azure_ad_token_from_entra_id = client_secret_grant
    try:
        yield
    finally:
        azure_common.get_azure_ad_token_from_entra_id = saved


@contextmanager
def global_settings(api_base: str | None, api_key: str | None) -> Generator[None]:
    saved_base = litellm.api_base
    saved_key = litellm.api_key
    saved_refresh = litellm.enable_azure_ad_token_refresh
    litellm.api_base = api_base
    litellm.api_key = api_key
    litellm.enable_azure_ad_token_refresh = False
    try:
        yield
    finally:
        litellm.api_base = saved_base
        litellm.api_key = saved_key
        litellm.enable_azure_ad_token_refresh = saved_refresh


def step_environment(step: UrlStep | AuthStep) -> dict[str, str]:
    values = step.get("environment")
    return {} if values is None else values


def url_answer(step: UrlStep) -> Object:
    with mock.patch.dict(os.environ, step_environment(step)), global_settings(
        step.get("global_api_base"), None
    ):
        try:
            url = RESPONSES.get_complete_url(step.get("api_base"), {})
        except (ValueError, httpx.InvalidURL):
            return {"kind": "error"}
    return {"kind": step["kind"], "url": url}


def auth_params(step: AuthStep) -> GenericLiteLLMParams:
    params: EntraParams = step.get("params", {})
    return GenericLiteLLMParams(
        **params, api_key=step.get("api_key"), api_base=step.get("api_base")
    )


def auth_answer(step: AuthStep) -> Object:
    given = step.get("headers")
    with (
        mock.patch.dict(os.environ, step_environment(step)),
        global_settings(step.get("global_api_base"), step.get("global_api_key")),
        grant_overrides(),
    ):
        try:
            validated = RESPONSES.validate_environment(
                {} if given is None else given, MODEL, auth_params(step)
            )
        except (TypeError, ValueError):
            return {"kind": "error"}
    return {"kind": "auth", "headers": JSON.validate_python(validated)}


def websocket_answer() -> Object:
    return {
        "kind": "no_native_websocket",
        "provider": RESPONSES.custom_llm_provider.name.lower(),
        "websocket": RESPONSES.supports_native_websocket(),
    }


def decisions_answer(step: DecisionsStep) -> Object:
    config = AzureAIDecisionsConfig()
    api_base = step.get("api_base")
    url = (
        None
        if api_base is None
        else config.get_complete_url(api_base=api_base, model=step["model"])
    )
    return {
        "kind": "decisions_route",
        "url": url,
        "api_base": config.resolve_api_base(api_base),
        "api_key": config.resolve_api_key(step.get("api_key")),
    }


def run(input_value: Input) -> list[Object]:
    answers: list[Object] = []
    for step in input_value["steps"]:
        match step["kind"]:
            case "auth":
                answers.append(auth_answer(step))
            case "decisions_route":
                answers.append(decisions_answer(step))
            case "no_native_websocket":
                answers.append(websocket_answer())
            case _:
                answers.append(url_answer(step))
    return answers
