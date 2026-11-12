import io
from collections.abc import Callable
from contextlib import redirect_stdout
from typing import NotRequired, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.azure.responses.transformation import AzureOpenAIResponsesAPIConfig
from litellm.llms.openai.responses import transformation
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

litellm.suppress_debug_info = True


class Probe(TypedDict):
    model: NotRequired[str]
    provider: NotRequired[str]
    params: NotRequired[Object]
    model_cost: NotRequired[dict[str, Json]]
    drop_params: NotRequired[bool]
    global_drop_params: NotRequired[bool]
    body: NotRequired[Object]


class Probes(TypedDict):
    probes: list[Probe]


Handler: TypeAlias = Callable[[Probe], Json]


def install(probe: Probe) -> None:
    given_cost = probe.get("model_cost")
    costs: dict[str, Json] = {} if given_cost is None else given_cost
    litellm.model_cost = costs
    global_drop = probe.get("global_drop_params")
    litellm.drop_params = False if global_drop is None else global_drop


def model_of(probe: Probe) -> str:
    model = probe.get("model")
    return "" if model is None else model


def params_of(probe: Probe) -> Object:
    params = probe.get("params")
    return {} if params is None else params


def drop_of(probe: Probe) -> bool:
    value = probe.get("drop_params")
    return False if value is None else value


def map_value(probe: Probe) -> Json:
    install(probe)
    config = config_of(probe)
    try:
        mapped = config.map_openai_params(params_of(probe), model_of(probe), drop_of(probe))  # type: ignore[misc, arg-type]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L207
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": error.status_code}
    return cast("Object", mapped)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L281


def supported_value(probe: Probe) -> Json:
    config = config_of(probe)
    supported = cast("list[str]", config.get_supported_openai_params(model=model_of(probe)))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L188
    return cast("Json", {"supported": sorted(supported)})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L188


def config_of(probe: Probe) -> OpenAIResponsesAPIConfig:
    provider = probe.get("provider")
    if provider is None or provider == "openai":
        return OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L105
    return AzureOpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/responses/transformation.py#L25


def capability_value(probe: Probe) -> Json:
    config = config_of(probe)
    value: Object = {
        "custom_llm_provider": config.custom_llm_provider.value,
        "supports_native_file_search": config.supports_native_file_search(),
        "supports_encrypted_agent_messages": config.supports_encrypted_agent_messages(),
    }
    body = probe.get("body")
    if body is not None:
        response = httpx.Response(200, json=body)
        object_body = transformation._json_object_body(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L100
        value["json_object_body"] = cast("Json", object_body)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L100
        delete_body = transformation._delete_response_body(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L95
        value["delete_response_body"] = cast("Json", delete_body)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L95
    return value


HANDLERS: dict[str, Handler] = {
    "minimum_output_tokens": map_value,
    "reasoning_unsupported_drop_or_error": map_value,
    "gpt5_temperature_default_effort": map_value,
    "top_p_with_reasoning": map_value,
    "unknown_catalog_model": map_value,
    "reasoning_summary_without_effort": map_value,
    "responses_parameter_set": supported_value,
    "provider_capabilities": capability_value,
}


def run(input_value: dict[str, Probes]) -> Json:
    if len(input_value) != 1:
        raise ValueError("expected one openai_response_params case")
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_response_params case: {name}")
    with redirect_stdout(io.StringIO()):
        return [handler(probe) for probe in input_value[name]["probes"]]
