import datetime as dt
from collections.abc import Callable
from typing import Final, Literal, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.nvidia_nim.rerank.common_utils import get_nvidia_nim_rerank_config
from litellm.llms.nvidia_nim.rerank.ranking_transformation import NvidiaNimRankingConfig
from litellm.llms.nvidia_nim.rerank.transformation import NvidiaNimRerankConfig
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue

from .config import JSON, Document, JsonObject, Kind, RerankConfig, set_env

FIXED_NOW: Final = dt.datetime(2026, 1, 1, tzinfo=dt.UTC)
ERRORS: Final = (AttributeError, IndexError, KeyError, TypeError, ValueError)
SELECTOR: Final = cast(Callable[[str], RerankConfig], get_nvidia_nim_rerank_config)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/rerank/common_utils.py#L6


class RerankUrlCase(TypedDict):
    model: str
    api_base: str | None


class RerankSelectInput(TypedDict):
    kind: Literal["rerank_select"]
    models: list[str]
    urls: list[RerankUrlCase]


class RerankCase(TypedDict):
    config: Kind
    model: str
    non_default: JsonObject
    top_n: int | None
    query: str
    documents: list[Document]


class HeaderCase(TypedDict):
    api_key: str | None
    env_key: str | None
    global_key: str | None
    headers: dict[str, str]


class RerankParamsInput(TypedDict):
    kind: Literal["rerank_params"]
    cases: list[RerankCase]
    headers: list[HeaderCase]


class RerankRequestInput(TypedDict):
    kind: Literal["rerank_request"]
    cases: list[RerankCase]


class RerankResponseCase(TypedDict):
    config: Kind
    model: str
    status: int
    body: str
    headers: dict[str, str]
    optional: JsonObject
    request_data: JsonObject


class RerankResponseInput(TypedDict):
    kind: Literal["rerank_response"]
    cases: list[RerankResponseCase]


def provider(kind: Kind) -> RerankConfig:
    # A fresh config per call: the ranking config keeps top_n on the instance
    if kind == "ranking":
        return NvidiaNimRankingConfig()
    return NvidiaNimRerankConfig()


def rerank_select(input_value: RerankSelectInput) -> JsonValue:
    selected = [
        SELECTOR(model).get_complete_url(None, model) for model in input_value["models"]
    ]
    urls = [
        SELECTOR(case["model"]).get_complete_url(case["api_base"], case["model"])
        for case in input_value["urls"]
    ]
    return {"selected": JSON.validate_python(selected), "urls": JSON.validate_python(urls)}


def mapped_case_params(config: RerankConfig, case: RerankCase) -> JsonObject:
    return config.map_cohere_rerank_params(
        non_default_params=case["non_default"],
        model=case["model"],
        drop_params=False,
        query=case["query"],
        documents=case["documents"],
        top_n=case["top_n"],
    )


def rerank_params(input_value: RerankParamsInput) -> JsonValue:
    mapped = [mapped_case_params(provider(case["config"]), case) for case in input_value["cases"]]
    headers: list[JsonValue] = []
    for case in input_value["headers"]:
        set_env("NVIDIA_NIM_API_KEY", case["env_key"])
        litellm.api_key = case["global_key"]
        try:
            merged = provider("retrieval").validate_environment(
                dict(case["headers"]), "meta/llama", case["api_key"]
            )
        except ValueError:
            headers.append({"ok": False})
            continue
        headers.append({"ok": True, "headers": JSON.validate_python(merged)})
    set_env("NVIDIA_NIM_API_KEY", None)
    litellm.api_key = None
    return {"mapped": JSON.validate_python(mapped), "headers": headers}


def rerank_request(input_value: RerankRequestInput) -> JsonValue:
    bodies: list[JsonValue] = []
    for case in input_value["cases"]:
        config = provider(case["config"])
        optional = mapped_case_params(config, case)
        try:
            body = config.transform_rerank_request(case["model"], optional, {})
        except ValueError:
            bodies.append({"ok": False})
            continue
        bodies.append({"ok": True, "body": body})
    return {"bodies": bodies}


def rerank_response(input_value: RerankResponseInput) -> JsonValue:
    responses: list[JsonValue] = []
    for case in input_value["cases"]:
        try:
            response = provider(case["config"]).transform_rerank_response(
                model=case["model"],
                raw_response=httpx.Response(
                    case["status"],
                    content=case["body"].encode(),
                    headers=dict(case["headers"]),
                    request=httpx.Request("POST", "https://ai.api.nvidia.com"),
                ),
                model_response=RerankResponse(),
                logging_obj=Logging(
                    model=case["model"],
                    messages=None,
                    stream=False,
                    call_type="arerank",
                    start_time=FIXED_NOW,
                    litellm_call_id="b5837",
                    function_id="b5837",
                ),
                request_data=case["request_data"],
                optional_params=case["optional"],
            )
        except ERRORS:
            responses.append({"ok": False})
            continue
        except BaseLLMException:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41
            responses.append({"ok": False})
            continue
        responses.append(JSON.validate_json(response.model_dump_json()))
    return {"responses": responses}
