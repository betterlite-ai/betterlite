import io
from contextlib import redirect_stdout
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
Provider: TypeAlias = Literal["openai", "azure", "hosted_vllm"]

litellm.suppress_debug_info = True

BODY: Final = TypeAdapter(JsonValue)


class ResponsesTransform(Protocol):
    # ast-grep-ignore: no-long-names
    def transform_responses_api_request(
        self,
        model: str,
        input_value: str | ResponseInputParam,
        params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...


class Probe(TypedDict):
    model: str
    input: str | ResponseInputParam
    params: NotRequired[Object]
    provider: NotRequired[Provider]
    model_info: NotRequired[Object | None]


class Probes(TypedDict):
    probes: list[Probe]


def responses_config(provider: LlmProviders) -> BaseResponsesAPIConfig:
    config = ProviderConfigManager.get_provider_responses_api_config(provider)
    if config is None:
        raise ValueError(f"no Responses API config for {provider.value}")
    return config


def config_of(probe: Probe) -> BaseResponsesAPIConfig:
    provider = probe.get("provider")
    if provider is None or provider == "openai":
        return responses_config(LlmProviders.OPENAI)
    if provider == "azure":
        return responses_config(LlmProviders.AZURE)
    return responses_config(LlmProviders.HOSTED_VLLM)


def params_of(probe: Probe) -> Object:
    params = probe.get("params")
    return {} if params is None else dict(params)


def transform_body(config: ResponsesTransform, probe: Probe) -> JsonValue:
    headers: dict[str, str] = {}
    litellm_params = GenericLiteLLMParams(model_info=probe.get("model_info"))
    body = config.transform_responses_api_request(
        probe["model"], probe["input"], params_of(probe), litellm_params, headers
    )
    return BODY.validate_python(body)


def run(input_value: Probes) -> JsonValue:
    with redirect_stdout(io.StringIO()):
        return [
            transform_body(config_of(probe), probe) for probe in input_value["probes"]
        ]
