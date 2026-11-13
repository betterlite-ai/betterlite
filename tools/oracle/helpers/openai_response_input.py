import io
from contextlib import redirect_stdout
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.azure.responses.transformation import AzureOpenAIResponsesAPIConfig
from litellm.llms.hosted_vllm.responses.transformation import HostedVLLMResponsesAPIConfig
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from openai.types.responses.response_input_param import ResponseInputParam

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Provider: TypeAlias = Literal["openai", "azure", "hosted_vllm"]

litellm.suppress_debug_info = True


class Probe(TypedDict):
    model: str
    input: str | ResponseInputParam
    params: NotRequired[Object]
    provider: NotRequired[Provider]
    model_info: NotRequired[Object | None]


class Probes(TypedDict):
    probes: list[Probe]


def config_of(probe: Probe) -> OpenAIResponsesAPIConfig:
    provider = probe.get("provider")
    if provider is None or provider == "openai":
        return OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L105
    if provider == "azure":
        return AzureOpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/responses/transformation.py#L25
    return HostedVLLMResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/hosted_vllm/responses/transformation.py#L17


def params_of(probe: Probe) -> Object:
    params = probe.get("params")
    return {} if params is None else dict(params)


def run_probe(probe: Probe) -> Json:
    config = config_of(probe)
    headers: dict[str, str] = {}
    litellm_params = GenericLiteLLMParams(model_info=probe.get("model_info"))
    body = config.transform_responses_api_request(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L311
        model=probe["model"],
        input=probe["input"],
        response_api_optional_request_params=params_of(probe),
        litellm_params=litellm_params,
        headers=headers,
    )
    return cast("Json", body)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L311


def run(input_value: Probes) -> Json:
    with redirect_stdout(io.StringIO()):
        return [run_probe(probe) for probe in input_value["probes"]]
