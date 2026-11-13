import datetime as dt
import os
from typing import Protocol, TypeAlias, TypedDict, Unpack

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.nvidia_nim.rerank.common_utils import get_nvidia_nim_rerank_config
from litellm.llms.nvidia_nim.rerank.ranking_transformation import NvidiaNimRankingConfig
from litellm.llms.nvidia_nim.rerank.transformation import NvidiaNimRerankConfig
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter

from .types import (
    Document,
    JsonObject,
    Kind,
    RerankCase,
    RerankParams,
    RerankRequest,
    RerankResponseCase,
    RerankSelect,
    ResponseCase,
)

ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
FIXED_NOW = dt.datetime(2026, 1, 1, tzinfo=dt.UTC)
ERRORS = (AttributeError, IndexError, KeyError, TypeError, ValueError, BaseLLMException)
ProviderConfig: TypeAlias = NvidiaNimRerankConfig | NvidiaNimRankingConfig


class MapArguments(TypedDict):
    non_default_params: JsonObject | None
    model: str
    drop_params: bool
    query: str
    documents: list[Document]
    top_n: int | None


class RerankConfig(Protocol):
    def get_complete_url(self, api_base: str | None, model: str) -> str: ...
    def map_cohere_rerank_params(self, **arguments: Unpack[MapArguments]) -> JsonObject: ...
    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...
    def transform_rerank_request(
        self, model: str, optional_rerank_params: JsonObject, headers: dict[str, str]
    ) -> JsonObject: ...
    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Logging,
        request_data: JsonObject,
    ) -> RerankResponse: ...


RERANK: RerankConfig = NvidiaNimRerankConfig()
RANKING: RerankConfig = NvidiaNimRankingConfig()


def provider(kind: Kind) -> ProviderConfig:
    if kind == "ranking":
        return RANKING
    return RERANK


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def mapped_params(config: ProviderConfig, call: RerankCase) -> JsonObject:
    non_default = None if call["non_default"] is None else dict(call["non_default"])
    arguments = MapArguments(
        non_default_params=non_default,
        model=call["model"],
        drop_params=False,
        query=call["query"],
        documents=call["documents"],
        top_n=call["top_n"],
    )
    return config.map_cohere_rerank_params(**arguments)


def rerank_select(input_value: RerankSelect) -> JsonValue:
    selected = [
        get_nvidia_nim_rerank_config(model).get_complete_url(None, model)
        for model in input_value["models"]
    ]
    urls = [
        get_nvidia_nim_rerank_config(call["model"]).get_complete_url(
            call["api_base"], call["model"]
        )
        for call in input_value["urls"]
    ]
    return ADAPTER.validate_python({"selected": selected, "urls": urls})


def rerank_params(input_value: RerankParams) -> JsonValue:
    mapped = [mapped_params(provider(call["config"]), call) for call in input_value["cases"]]
    results: list[JsonValue] = []
    for call in input_value["headers"]:
        set_env("NVIDIA_NIM_API_KEY", call["env_key"])
        litellm.api_key = call["global_key"]
        try:
            headers = RERANK.validate_environment(
                headers=dict(call["headers"]), model="meta/llama", api_key=call["api_key"]
            )
        except ValueError:
            results.append({"ok": False})
            continue
        results.append({"ok": True, "headers": headers})
    set_env("NVIDIA_NIM_API_KEY", None)
    litellm.api_key = None
    return ADAPTER.validate_python({"mapped": mapped, "headers": results})


def rerank_request(input_value: RerankRequest) -> JsonValue:
    bodies: list[JsonValue] = []
    for call in input_value["cases"]:
        config = provider(call["config"])
        optional = mapped_params(config, call)
        try:
            body = config.transform_rerank_request(
                model=call["model"], optional_rerank_params=optional, headers={}
            )
        except ValueError:
            bodies.append({"ok": False})
            continue
        bodies.append({"ok": True, "body": body})
    return ADAPTER.validate_python({"bodies": bodies})


def one_response(call: ResponseCase) -> JsonValue:
    config = provider(call["config"])
    raw = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=dict(call["headers"]),
        request=httpx.Request("POST", "https://ai.api.nvidia.com"),
    )
    logging_obj = Logging(
        model=call["model"],
        messages=None,
        stream=False,
        call_type="arerank",
        start_time=FIXED_NOW,
        litellm_call_id="b5837",
        function_id="b5837",
    )
    try:
        response = config.transform_rerank_response(
            model=call["model"],
            raw_response=raw,
            model_response=RerankResponse(),
            logging_obj=logging_obj,
            request_data=dict(call["request_data"]),
        )
    except ERRORS:
        return {"ok": False}
    return ADAPTER.validate_json(response.model_dump_json())


def rerank_response(input_value: RerankResponseCase) -> JsonValue:
    responses = [one_response(call) for call in input_value["cases"]]
    return ADAPTER.validate_python({"responses": responses})


def run_rerank(
    input_value: RerankSelect | RerankParams | RerankRequest | RerankResponseCase,
) -> JsonValue:
    match input_value["kind"]:
        case "rerank_select":
            return rerank_select(input_value)
        case "rerank_params":
            return rerank_params(input_value)
        case "rerank_request":
            return rerank_request(input_value)
        case "rerank_response":
            return rerank_response(input_value)
