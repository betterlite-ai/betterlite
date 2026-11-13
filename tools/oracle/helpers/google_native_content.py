from dataclasses import dataclass
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.google_genai.main import GenerateContentResponse
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Provider: TypeAlias = Literal["studio", "vertex"]


# the native config's interface, so the case input stays JsonValue and the calls keep their keys
class NativeConfig(Protocol):
    def map_generate_content_optional_params(  # ast-grep-ignore: no-long-names
        self, generate_content_config_dict: Object, model: str
    ) -> Object: ...

    def transform_generate_content_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        contents: JsonValue,
        tools: JsonValue | None,
        generate_content_config_dict: Object,
        system_instruction: JsonValue | None = None,
    ) -> Object: ...

    def transform_generate_content_response(  # ast-grep-ignore: no-long-names
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> GenerateContentResponse: ...


@dataclass
class RecordedLogging:
    model_call_details: dict[str, JsonValue]


class OptionalParamsCall(TypedDict):
    function: Literal["optional_params"]
    provider: Provider
    model: str
    config: Object


class RequestCall(TypedDict):
    function: Literal["request"]
    provider: Provider
    model: str
    contents: JsonValue
    generation_config: Object
    settings: Headers
    tools: NotRequired[JsonValue]
    system_instruction: NotRequired[JsonValue]


class ResponseCall(TypedDict):
    function: Literal["response"]
    provider: Provider
    model: str
    status: int
    body: str
    headers: NotRequired[Headers]


Call: TypeAlias = OptionalParamsCall | RequestCall | ResponseCall


class Input(TypedDict):
    calls: list[Call]


INPUT = TypeAdapter(Input)
JSON = TypeAdapter(JsonValue)
PROVIDERS: Final = {"studio": LlmProviders.GEMINI, "vertex": LlmProviders.VERTEX_AI}


def selected_config(provider: Provider, model: str) -> NativeConfig:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        model, PROVIDERS[provider]
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {model}")
    return selected


def optional_params(call: OptionalParamsCall) -> JsonValue:
    config = selected_config(call["provider"], call["model"])
    mapped = config.map_generate_content_optional_params(call["config"], call["model"])
    return {"config": mapped}


def request_body(call: RequestCall) -> JsonValue:
    config = selected_config(call["provider"], call["model"])
    return config.transform_generate_content_request(
        model=call["model"],
        contents=call["contents"],
        tools=call.get("tools"),
        generate_content_config_dict=call["generation_config"],
        system_instruction=call.get("system_instruction"),
    )


def response_record(call: ResponseCall) -> JsonValue:
    config = selected_config(call["provider"], call["model"])
    reply = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode("utf-8")
    )
    logging = RecordedLogging(model_call_details={})
    response = config.transform_generate_content_response(
        model=call["model"], raw_response=reply, logging_obj=logging  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/google_genai/transformation.py#L362
    )
    dumped: str = response.model_dump_json()
    return JSON.validate_json(dumped)


def resolve(call: Call) -> JsonValue:
    if call["function"] == "optional_params":
        return optional_params(call)
    if call["function"] == "request":
        return request_body(call)
    return response_record(call)


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
