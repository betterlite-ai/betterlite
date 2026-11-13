import os
from typing import Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.openrouter.chat.transformation import OpenrouterConfig
from litellm.llms.openrouter.decisions.transformation import OpenRouterDecisionsConfig
from litellm.llms.openrouter.embedding.transformation import OpenrouterEmbeddingConfig
from litellm.llms.openrouter.responses.transformation import OpenRouterResponsesAPIConfig
from litellm.types.decisions import DecisionsIRResponse, DecisionsIRUsage
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from pydantic import JsonValue, TypeAdapter

MODEL = "m"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException: ...


class EmbeddingConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...


class ResponsesConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        litellm_params: GenericLiteLLMParams | None,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def supports_native_websocket(self) -> bool: ...

    @property
    def custom_llm_provider(self) -> LlmProviders: ...


CHAT: ChatConfig = OpenrouterConfig()
EMBEDDING: EmbeddingConfig = OpenrouterEmbeddingConfig()
RESPONSES: ResponsesConfig = OpenRouterResponsesAPIConfig()
DECISIONS = OpenRouterDecisionsConfig()


class SupportedInput(TypedDict):
    op: Literal["supported"]
    models: list[str]


class EmbeddingInput(TypedDict):
    op: Literal["embedding"]


class ResponsesCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    global_api_key: str | None
    environment: dict[str, str]
    api_base: str | None
    global_api_base: str | None


class ResponsesInput(TypedDict):
    op: Literal["responses"]
    cases: list[ResponsesCase]


class DecisionsCase(TypedDict):
    cost: JsonValue


class DecisionsInput(TypedDict):
    op: Literal["decisions"]
    cases: list[DecisionsCase]


class ErrorCase(TypedDict):
    message: str
    status: int
    headers: dict[str, str]


class ErrorInput(TypedDict):
    op: Literal["error_class"]
    cases: list[ErrorCase]


Input: TypeAlias = (
    SupportedInput | EmbeddingInput | ResponsesInput | DecisionsInput | ErrorInput
)


def responses(case: ResponsesCase) -> JsonValue:
    for name in ("OPENROUTER_API_KEY", "OR_API_KEY", "OPENROUTER_API_BASE"):
        os.environ.pop(name, None)
    for name, value in case["environment"].items():
        os.environ[name] = value
    saved_key = litellm.api_key
    saved_base = litellm.api_base
    litellm.api_key = case["global_api_key"]
    litellm.api_base = case["global_api_base"]
    try:
        params = GenericLiteLLMParams(api_key=case["api_key"])
        headers = RESPONSES.validate_environment(dict(case["headers"]), MODEL, params)
        url = RESPONSES.get_complete_url(case["api_base"], {})
    finally:
        litellm.api_key = saved_key
        litellm.api_base = saved_base
    result: dict[str, JsonValue] = {
        "headers": headers,
        "url": url,
        "provider": RESPONSES.custom_llm_provider.name.lower(),
        "websocket": RESPONSES.supports_native_websocket(),
    }
    return result


def decisions_cost(case: DecisionsCase) -> float | None:
    usage = DecisionsIRUsage(extra={"cost": case["cost"]})
    response = DecisionsIRResponse(model=None, answers=(), usage=usage)
    return DECISIONS.provider_reported_cost(response)


def decisions(cases: list[DecisionsCase]) -> JsonValue:
    costs = [decisions_cost(case) for case in cases]
    return JSON.validate_python(
        {
            "path": DECISIONS.path,
            "api_key_env": list(DECISIONS.api_key_env),
            "api_base_env": list(DECISIONS.api_base_env),
            "default_api_base": DECISIONS.get_default_api_base(),
            "complete_url": DECISIONS.get_complete_url("https://openrouter.ai/api/v1", MODEL),
            "costs": costs,
        }
    )


def error_class(case: ErrorCase) -> JsonValue:
    error = CHAT.get_error_class(case["message"], case["status"], dict(case["headers"]))
    result: dict[str, JsonValue] = {"message": error.message, "status": error.status_code}
    return result


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "supported":
        return JSON.validate_python(
            [CHAT.get_supported_openai_params(model) for model in input_value["models"]]
        )
    if input_value["op"] == "embedding":
        return JSON.validate_python(EMBEDDING.get_supported_openai_params(MODEL))
    if input_value["op"] == "responses":
        return JSON.validate_python([responses(case) for case in input_value["cases"]])
    if input_value["op"] == "decisions":
        return decisions(input_value["cases"])
    return JSON.validate_python([error_class(case) for case in input_value["cases"]])
