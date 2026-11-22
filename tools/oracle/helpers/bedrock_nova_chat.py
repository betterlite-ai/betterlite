import datetime as dt
import os
from collections.abc import Callable
from typing import Literal, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.amazon_nova.chat.transformation import AmazonNovaChatConfig
from litellm.llms.bedrock.chat.invoke_transformations.amazon_nova_transformation import (
    AmazonInvokeNovaConfig,
)
from litellm.types.llms.bedrock import BedrockInvokeNovaRequest, MessageBlock, SystemContentBlock
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue as Json, TypeAdapter


class Input(TypedDict, total=False):
    function: Literal[
        "inline_cache_points",
        "filter_allowed_fields",
        "tool_config_injection",
        "endpoint",
        "converse_delegation",
        "defaults",
        "response_model",
    ]
    system: list[SystemContentBlock]
    messages: list[MessageBlock]
    model: str
    model_cost: dict[str, Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    api_base: str | None
    api_key: str | None
    global_api_key: str | None
    litellm_api_key: str | None
    environment: dict[str, str]
    non_default_params: dict[str, Json]
    arguments: dict[str, Json]
    body: str


class InlineCase(TypedDict):
    system: list[SystemContentBlock]
    messages: list[MessageBlock]


class TransformCase(TypedDict):
    model: str
    messages: list[Json]
    model_cost: dict[str, Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]


class EndpointCase(TypedDict):
    api_base: str | None
    api_key: str | None
    global_api_key: str | None
    litellm_api_key: str | None
    environment: dict[str, str]


class DelegationCase(TypedDict):
    model: str
    model_cost: dict[str, Json]
    non_default_params: dict[str, Json]
    optional_params: dict[str, Json]


class Arguments(TypedDict, total=False):
    max_completion_tokens: int
    max_tokens: int
    temperature: int
    top_p: int
    tools: list[Json]
    reasoning_effort: list[Json]


class ResponseCase(TypedDict):
    model: str
    body: str


JSON: TypeAdapter[Json] = TypeAdapter(Json)
REQUEST: TypeAdapter[BedrockInvokeNovaRequest] = TypeAdapter(BedrockInvokeNovaRequest)
INLINE: TypeAdapter[InlineCase] = TypeAdapter(InlineCase)
TRANSFORM: TypeAdapter[TransformCase] = TypeAdapter(TransformCase)
ENDPOINT: TypeAdapter[EndpointCase] = TypeAdapter(EndpointCase)
DELEGATION: TypeAdapter[DelegationCase] = TypeAdapter(DelegationCase)
ARGUMENTS: TypeAdapter[Arguments] = TypeAdapter(Arguments)
RESPONSE: TypeAdapter[ResponseCase] = TypeAdapter(ResponseCase)


# LiteLLM's methods take bare dicts, so each seam is read with the JSON-facing signature.
@runtime_checkable
class Transformer(Protocol):
    transform_request: Callable[
        [str, list[Json], dict[str, Json], dict[str, Json], dict[str, Json]], Json
    ]
    map_openai_params: Callable[[dict[str, Json], dict[str, Json], str, bool], dict[str, Json]]


@runtime_checkable
class DirectWire(Protocol):
    transform_response: Callable[
        [
            str,
            httpx.Response,
            ModelResponse,
            Logging,
            dict[str, Json],
            list[Json],
            dict[str, Json],
            dict[str, Json],
            None,
        ],
        ModelResponse,
    ]


class TransformerSeam(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    seam: Transformer


class DirectSeam(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    seam: DirectWire


# Both class methods are untyped upstream, so the protocol carries their signatures.
class Declared(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, Json]: ...


DECLARED: type[Declared] = AmazonNovaChatConfig


# The private cache-point pass is reachable through a subclass, the seam ruff and pyright allow.
class InvokeConfig(AmazonInvokeNovaConfig):
    def inline(self, request: BedrockInvokeNovaRequest) -> Json:
        return JSON.validate_python(self._inline_cache_points(request))

    def supported(self, model: str) -> list[str]:
        return self.get_supported_openai_params(model)


class DirectConfig(AmazonNovaChatConfig):
    def endpoint(self, api_base: str | None, api_key: str | None) -> Json:
        base, key = self._get_openai_compatible_provider_info(api_base, api_key)
        return JSON.validate_python({"api_base": base, "api_key": key})


def logging_fixture() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def inline_points(case: InlineCase) -> Json:
    request = REQUEST.validate_python({"system": case["system"], "messages": case["messages"]})
    return InvokeConfig().inline(request)


def invoke_body(case: TransformCase) -> Json:
    litellm.model_cost = case["model_cost"]
    seam = TransformerSeam.model_validate({"seam": InvokeConfig()}).seam
    body = seam.transform_request(
        case["model"],
        list(case["messages"]),
        dict(case["optional_params"]),
        dict(case["litellm_params"]),
        {},
    )
    return JSON.validate_python({"body": body})


def endpoint(case: EndpointCase) -> Json:
    litellm.amazon_nova_api_key = case["global_api_key"]
    litellm.api_key = case["litellm_api_key"]
    # the oracle runs one call per process, so the call's own variables are the whole environment
    os.environ.update(case["environment"])
    return DirectConfig().endpoint(case["api_base"], case["api_key"])


def delegation(case: DelegationCase) -> Json:
    litellm.model_cost = case["model_cost"]
    config = InvokeConfig()
    seam = TransformerSeam.model_validate({"seam": config}).seam
    drop_params = False
    mapped = seam.map_openai_params(
        dict(case["non_default_params"]),
        dict(case["optional_params"]),
        case["model"],
        drop_params,
    )
    return JSON.validate_python({"supported": config.supported(case["model"]), "params": mapped})


def defaults(case: Arguments) -> Json:
    # the constructor writes its non-None fields onto the class dict get_config reads
    _ = AmazonNovaChatConfig(
        max_completion_tokens=case.get("max_completion_tokens"),
        max_tokens=case.get("max_tokens"),
        temperature=case.get("temperature"),
        top_p=case.get("top_p"),
        tools=case.get("tools"),
        reasoning_effort=case.get("reasoning_effort"),
    )
    return JSON.validate_python(DECLARED.get_config())


def response_model(case: ResponseCase) -> Json:
    seam = DirectSeam.model_validate({"seam": DirectConfig()}).seam
    reply = seam.transform_response(
        case["model"],
        httpx.Response(200, content=case["body"].encode()),
        ModelResponse(),
        logging_fixture(),
        {},
        [],
        {},
        {},
        None,
    )
    return JSON.validate_python({"model": reply.model})


def answer(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "inline_cache_points":
        return inline_points(INLINE.validate_python(input_value))
    if function in ("filter_allowed_fields", "tool_config_injection"):
        return invoke_body(TRANSFORM.validate_python(input_value))
    if function == "endpoint":
        return endpoint(ENDPOINT.validate_python(input_value))
    if function == "converse_delegation":
        return delegation(DELEGATION.validate_python(input_value))
    if function == "defaults":
        return defaults(ARGUMENTS.validate_python(input_value["arguments"]))
    if function == "response_model":
        return response_model(RESPONSE.validate_python(input_value))
    raise ValueError("unknown bedrock_nova_chat function")


def run(input_value: Input) -> Json:
    # the proxy sets these; the drop and modify branches read them as globals
    litellm.suppress_debug_info = True
    litellm.drop_params = False
    litellm.modify_params = False
    litellm.bedrock_neutralize_orphaned_tool_blocks = False
    litellm.bedrock_request_metadata_fields = None
    return answer(input_value)
