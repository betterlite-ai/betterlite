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
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, TypeAdapter
from pydantic import JsonValue as Json


class Arguments(TypedDict, total=False):
    max_completion_tokens: int
    max_tokens: int
    temperature: int
    top_p: int
    tools: list[Json]
    reasoning_effort: list[Json]


class InlineCase(TypedDict):
    function: Literal["inline_cache_points"]
    system: list[Json]
    messages: list[Json]


class TransformCase(TypedDict):
    function: Literal["filter_allowed_fields", "tool_config_injection"]
    model: str
    messages: list[Json]
    model_cost: dict[str, Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]


class EndpointCase(TypedDict):
    function: Literal["endpoint"]
    api_base: str | None
    api_key: str | None
    global_api_key: str | None
    litellm_api_key: str | None
    environment: dict[str, str]


class DelegationCase(TypedDict):
    function: Literal["converse_delegation"]
    model: str
    model_cost: dict[str, Json]
    environment: dict[str, str]
    non_default_params: dict[str, Json]
    optional_params: dict[str, Json]


class DefaultsCase(TypedDict):
    function: Literal["defaults"]
    arguments: Arguments


class ResponseCase(TypedDict):
    function: Literal["response_model"]
    model: str
    body: str


Input = InlineCase | TransformCase | EndpointCase | DelegationCase | DefaultsCase | ResponseCase


class InvokeRequest(TypedDict, total=False):
    system: list[Json]
    messages: list[Json]


class Reply(TypedDict):
    model: str


JSON: TypeAdapter[Json] = TypeAdapter(Json)
REPLY: TypeAdapter[Reply] = TypeAdapter(Reply)


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
    def __init__(self) -> None:
        super().__init__()  # type: ignore[misc, no-untyped-call]  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_nova_transformation.py#L83-L84

    def inline(self, request: InvokeRequest) -> Json:
        return JSON.validate_python(self._inline_cache_points(request))  # type: ignore[arg-type, misc]  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_nova_transformation.py#L150-L170

    def supported(self, model: str) -> list[str]:
        return self.get_supported_openai_params(model)  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_nova_transformation.py#L86-L87


# The endpoint chain is typed upstream, so only the supported list needs the parent's seam.
class DirectConfig(AmazonNovaChatConfig):
    def endpoint(self, api_base: str | None, api_key: str | None) -> Json:
        base, key = self._get_openai_compatible_provider_info(api_base, api_key)
        resolved: dict[str, Json] = {"api_base": base, "api_key": key}
        return JSON.validate_python(resolved)

    def supported(self, model: str) -> list[str]:
        return self.get_supported_openai_params(model)  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/amazon_nova/chat/transformation.py#L64-L77


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
    request: InvokeRequest = {"system": case["system"], "messages": case["messages"]}
    return InvokeConfig().inline(request)


def invoke_body(case: TransformCase) -> Json:
    litellm.model_cost = case["model_cost"]
    holder: dict[str, InvokeConfig] = {"seam": InvokeConfig()}
    seam = TransformerSeam.model_validate(holder).seam
    body = seam.transform_request(
        case["model"],
        list(case["messages"]),
        dict(case["optional_params"]),
        dict(case["litellm_params"]),
        {},
    )
    answer: dict[str, Json] = {"body": body}
    return JSON.validate_python(answer)


def endpoint(case: EndpointCase) -> Json:
    litellm.amazon_nova_api_key = case["global_api_key"]
    litellm.api_key = case["litellm_api_key"]
    # the oracle runs one call per process, so the call's own variables are the whole environment
    os.environ.update(case["environment"])
    return DirectConfig().endpoint(case["api_base"], case["api_key"])


def delegation(case: DelegationCase) -> Json:
    litellm.model_cost = case["model_cost"]
    config = InvokeConfig()
    holder: dict[str, InvokeConfig] = {"seam": config}
    seam = TransformerSeam.model_validate(holder).seam
    drop_params = False
    mapped = seam.map_openai_params(
        dict(case["non_default_params"]),
        dict(case["optional_params"]),
        case["model"],
        drop_params,
    )
    supported = config.supported(case["model"])
    answer: dict[str, Json] = {"supported": supported, "params": mapped}
    return JSON.validate_python(answer)


def defaults(arguments: Arguments) -> Json:
    # the constructor writes its non-None fields onto the class dict get_config reads
    _ = AmazonNovaChatConfig(
        max_completion_tokens=arguments.get("max_completion_tokens"),
        max_tokens=arguments.get("max_tokens"),
        temperature=arguments.get("temperature"),
        top_p=arguments.get("top_p"),
        tools=arguments.get("tools"),
        reasoning_effort=arguments.get("reasoning_effort"),
    )
    rendered: dict[str, Json] = {
        "config": DECLARED.get_config(),
        "supported": DirectConfig().supported("amazon.nova-pro-v1:0"),
    }
    return JSON.validate_python(rendered)


def response_model(case: ResponseCase) -> Json:
    holder: dict[str, DirectConfig] = {"seam": DirectConfig()}
    seam = DirectSeam.model_validate(holder).seam
    returned = seam.transform_response(
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
    reply = REPLY.validate_json(returned.model_dump_json())
    answer: dict[str, Json] = {"model": reply["model"]}
    return JSON.validate_python(answer)


def run(input_value: Input) -> Json:
    # the proxy sets these; the drop and modify branches read them as globals
    litellm.suppress_debug_info = True
    litellm.drop_params = False
    litellm.modify_params = False
    litellm.bedrock_neutralize_orphaned_tool_blocks = False
    litellm.bedrock_request_metadata_fields = None
    if input_value["function"] == "inline_cache_points":
        return inline_points(input_value)
    if (
        input_value["function"] == "filter_allowed_fields"
        or input_value["function"] == "tool_config_injection"
    ):
        return invoke_body(input_value)
    if input_value["function"] == "endpoint":
        return endpoint(input_value)
    if input_value["function"] == "converse_delegation":
        return delegation(input_value)
    if input_value["function"] == "defaults":
        return defaults(input_value["arguments"])
    return response_model(input_value)
