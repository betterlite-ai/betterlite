from typing import Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue

HeaderMap: TypeAlias = dict[str, str]
Params: TypeAlias = dict[str, JsonValue]
ApiKey: TypeAlias = str | dict[str, str] | None


class Config(Protocol):
    def validate_environment(
        self,
        headers: HeaderMap | None,
        model: str,
        messages: list[AllMessageValues],
        optional_params: Params,
        litellm_params: Params,
        api_key: ApiKey = None,
        api_base: str | None = None,
    ) -> HeaderMap: ...


CONFIG: Config = VertexGeminiConfig()

litellm.suppress_debug_info = True


class Case(TypedDict):
    api_key: str | None
    auth_map: HeaderMap | None
    headers: HeaderMap | None
    optional_params: Params


class Answer(TypedDict):
    headers: dict[str, JsonValue]
    use_psc_endpoint_format: bool


def api_key(case: Case) -> ApiKey:
    # LiteLLM takes either the bearer string or the auth map in the same api_key parameter
    if case["auth_map"] is not None:
        return case["auth_map"]
    return case["api_key"]


def run(input_value: Case) -> Answer:
    optional_params = input_value["optional_params"]
    headers = CONFIG.validate_environment(
        input_value["headers"],
        "gemini-2.5-flash",
        [],
        optional_params,
        {},
        api_key=api_key(input_value),
        api_base=None,
    )
    requested = optional_params.get("use_psc_endpoint_format")
    return {
        "headers": dict[str, JsonValue](headers),
        "use_psc_endpoint_format": requested is True,
    }
