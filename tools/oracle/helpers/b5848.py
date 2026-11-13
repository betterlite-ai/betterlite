import importlib
import os
from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict
from unittest.mock import Mock

import fastuuid
import httpx
import litellm.litellm_core_utils.token_counter as token_counter_module
import litellm.llms.huggingface.rerank.transformation as rerank_transformation
import litellm.utils
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.huggingface.rerank.transformation import HuggingFaceRerankConfig
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter

REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

HF_BASE = "HF_API_BASE"
HUGGINGFACE_BASE = "HUGGINGFACE_API_BASE"

ApiBase = Callable[[str, str | None], str]
CompleteUrl = Callable[[str | None, str, dict[str, JsonValue] | None], str]
SupportedParams = Callable[[str], list[str]]
MapParams = Callable[
    [dict[str, JsonValue] | None, str, bool, str, list[str | dict[str, JsonValue]]],
    dict[str, JsonValue],
]
ValidateEnvironment = Callable[
    [
        dict[str, str],
        str,
        str | None,
        dict[str, JsonValue] | None,
        Mapping[str, JsonValue] | None,
        str | None,
    ],
    dict[str, str],
]
TransformRequest = Callable[
    [str, dict[str, JsonValue], dict[str, str], dict[str, JsonValue] | None],
    dict[str, JsonValue],
]
TransformResponse = Callable[
    [
        str,
        httpx.Response,
        RerankResponse,
        Logging,
        str | None,
        dict[str, JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
    ],
    RerankResponse,
]


class RerankConfig(Protocol):
    @property
    def get_api_base(self) -> ApiBase: ...

    @property
    def get_complete_url(self) -> CompleteUrl: ...

    @property
    def get_supported_cohere_rerank_params(self) -> SupportedParams: ...

    @property
    def map_cohere_rerank_params(self) -> MapParams: ...

    @property
    def validate_environment(self) -> ValidateEnvironment: ...

    @property
    def transform_rerank_request(self) -> TransformRequest: ...

    @property
    def transform_rerank_response(self) -> TransformResponse: ...


class UrlStep(TypedDict):
    kind: Literal["url"]
    model: str
    api_base: str | None
    env: dict[str, str]


class SupportedParamsStep(TypedDict):
    kind: Literal["supported_params"]
    model: str


class ParamsStep(TypedDict):
    kind: Literal["params"]
    model: str
    drop_params: bool
    query: str
    documents: list[str | dict[str, JsonValue]]
    custom_llm_provider: str | None
    top_n: int | None
    rank_fields: list[str] | None
    return_documents: bool | None
    max_chunks_per_doc: int | None
    max_tokens_per_doc: int | None
    instruction: str | None
    non_default_params: dict[str, JsonValue] | None


class ValidateEnvironmentStep(TypedDict):
    kind: Literal["validate_environment"]
    headers: dict[str, str]
    api_key: str | None
    api_base: str | None


class RequestStep(TypedDict):
    kind: Literal["request"]
    model: str
    params: dict[str, JsonValue]


class ResponseStep(TypedDict):
    kind: Literal["response"]
    model: str
    status: int
    body: str
    request_data: dict[str, JsonValue]
    token_counts: list[int | None] | None
    id: str


Step: TypeAlias = (
    UrlStep
    | SupportedParamsStep
    | ParamsStep
    | ValidateEnvironmentStep
    | RequestStep
    | ResponseStep
)


class Input(TypedDict):
    steps: list[Step]


STEPS: TypeAdapter[list[Step]] = TypeAdapter(list[Step])


class RecordedUuid4:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> fastuuid.UUID:
        return fastuuid.UUID(hex=self.value)


def environment(values: dict[str, str]) -> None:
    os.environ.pop(HF_BASE, None)
    os.environ.pop(HUGGINGFACE_BASE, None)
    for name, value in values.items():
        os.environ[name] = value


def url_value(step: UrlStep, view: RerankConfig) -> JsonValue:
    environment(step["env"])
    base = view.get_api_base(step["model"], step["api_base"])
    return {"base": base, "complete": view.get_complete_url(step["api_base"], step["model"], None)}


def supported_params_value(step: SupportedParamsStep, view: RerankConfig) -> JsonValue:
    return REPLY.validate_python(view.get_supported_cohere_rerank_params(step["model"]))


def params_value(step: ParamsStep, view: RerankConfig) -> JsonValue:
    nondefault = step["non_default_params"]
    drop = step["drop_params"]
    return REPLY.validate_python(
        view.map_cohere_rerank_params(
            {} if nondefault is None else nondefault,
            step["model"],
            drop,
            step["query"],
            step["documents"],
        )
    )


def validate_environment_value(step: ValidateEnvironmentStep, view: RerankConfig) -> JsonValue:
    return REPLY.validate_python(
        view.validate_environment(
            step["headers"],
            "model",
            step["api_key"],
            None,
            None,
            step["api_base"],
        )
    )


def request_value(step: RequestStep, view: RerankConfig) -> JsonValue:
    return REPLY.validate_python(
        view.transform_rerank_request(step["model"], step["params"], {}, None)
    )


def logging_object() -> Logging:
    return Logging(
        model="rerank",
        messages=None,
        stream=False,
        call_type="rerank",
        start_time=datetime.now(UTC),
        litellm_call_id="rerank-helper",
        function_id="rerank-helper",
    )


def real_token_counter(model: str = "", text: str | list[str] | None = None) -> int:
    return token_counter_module.token_counter(model=model, text=text)


def counter_mock(step: ResponseStep) -> Mock:
    outcomes = step["token_counts"]
    if outcomes is None:
        return Mock(side_effect=real_token_counter)
    effects: list[int | Exception] = [
        ValueError("the case records a token counting failure") if outcome is None else outcome
        for outcome in outcomes
    ]
    return Mock(side_effect=effects)


def patched_config(counter: Mock, request_id: str) -> HuggingFaceRerankConfig:
    litellm.utils.token_counter = counter
    fastuuid.uuid4 = RecordedUuid4(request_id)
    importlib.reload(rerank_transformation)
    return rerank_transformation.HuggingFaceRerankConfig()


def response_value(step: ResponseStep, view: RerankConfig) -> JsonValue:
    raw_response = httpx.Response(
        step["status"],
        content=step["body"].encode(),
        request=httpx.Request("POST", "https://huggingface.example/rerank"),
    )
    reply = view.transform_rerank_response(
        step["model"],
        raw_response,
        RerankResponse(),
        logging_object(),
        None,
        step["request_data"],
        {},
        {},
    )
    return REPLY.validate_json(reply.model_dump_json())


def run_step(step: Step, view: RerankConfig) -> JsonValue:
    if step["kind"] == "url":
        return url_value(step, view)
    if step["kind"] == "supported_params":
        return supported_params_value(step, view)
    if step["kind"] == "params":
        return params_value(step, view)
    if step["kind"] == "validate_environment":
        return validate_environment_value(step, view)
    if step["kind"] == "request":
        return request_value(step, view)
    return response_value(step, patched_config(counter_mock(step), step["id"]))


def run(input_value: Input) -> JsonValue:
    view: RerankConfig = HuggingFaceRerankConfig()
    return [run_step(step, view) for step in STEPS.validate_python(input_value["steps"])]
