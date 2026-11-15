from collections.abc import Callable
from typing import Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue

HeaderMap: TypeAlias = dict[str, str]
Params: TypeAlias = dict[str, JsonValue]
ApiKey: TypeAlias = str | dict[str, str] | None


class Config(Protocol):
    @property
    def validate_environment(
        self,
    ) -> Callable[
        [
            HeaderMap | None,
            str,
            list[AllMessageValues],
            Params,
            Params,
            ApiKey,
            str | None,
        ],
        HeaderMap,
    ]: ...


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
        [],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params,
        {},
        api_key(input_value),
        None,
    )
    requested = optional_params.get("use_psc_endpoint_format")
    return {
        "headers": dict[str, JsonValue](headers),
        "use_psc_endpoint_format": requested is True,
    }
