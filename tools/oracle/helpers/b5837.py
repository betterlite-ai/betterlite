import datetime as dt
import os
from collections.abc import Callable, Sequence
from typing import Final, Literal, Never, Protocol, TypeAlias, TypedDict, Unpack

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.nvidia_nim.chat.transformation import NvidiaNimConfig
from litellm.llms.nvidia_nim.embed import NvidiaNimEmbeddingConfig
from litellm.llms.nvidia_nim.passthrough.transformation import NvidiaNimPassthroughConfig
from litellm.llms.nvidia_nim.rerank.common_utils import get_nvidia_nim_rerank_config
from litellm.llms.nvidia_nim.rerank.ranking_transformation import NvidiaNimRankingConfig
from litellm.llms.nvidia_nim.rerank.transformation import NvidiaNimRerankConfig
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
Document: TypeAlias = str | dict[str, object]
Kind: TypeAlias = Literal["retrieval", "ranking"]
Operation: TypeAlias = Literal[
    "chat_supported",
    "chat_map",
    "embedding_map",
    "rerank_select",
    "rerank_params",
    "rerank_request",
    "rerank_response",
    "passthrough_url",
    "passthrough_credentials",
]
JSON: Final = TypeAdapter(JsonValue)
FIXED_NOW: Final = dt.datetime(2026, 1, 1, tzinfo=dt.UTC)
ERRORS: Final = (AttributeError, IndexError, KeyError, TypeError, ValueError)


class MapArguments(TypedDict):
    non_default_params: JsonObject
    model: str
    drop_params: bool
    query: str
    documents: list[Document]
    top_n: int | None


class ResponseArguments(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: RerankResponse
    logging_obj: Logging
    api_key: str | None
    request_data: JsonObject
    optional_params: JsonObject


class UrlArguments(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    endpoint: str
    request_query_params: JsonObject | None
    litellm_params: JsonObject


class EnvironmentArguments(TypedDict):
    headers: dict[str, str]
    model: str
    messages: Sequence[Never]
    optional_params: JsonObject
    litellm_params: JsonObject
    api_key: str | None
    api_base: str | None


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str, /) -> list[str]: ...
    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...


class EmbeddingConfig(Protocol):
    def get_supported_openai_params(self, /) -> list[str]: ...
    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        kwargs: JsonObject | None,
        /,
    ) -> JsonObject: ...


class RerankConfig(Protocol):
    def get_complete_url(self, api_base: str | None, model: str, /) -> str: ...
    def map_cohere_rerank_params(self, **arguments: Unpack[MapArguments]) -> JsonObject: ...
    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None, /
    ) -> dict[str, str]: ...
    def transform_rerank_request(
        self, model: str, optional_rerank_params: JsonObject, headers: dict[str, str], /
    ) -> JsonObject: ...
    def transform_rerank_response(
        self, **arguments: Unpack[ResponseArguments]
    ) -> RerankResponse: ...


class PassthroughConfig(Protocol):
    def get_complete_url(self, **arguments: Unpack[UrlArguments]) -> tuple[httpx.URL, str]: ...
    def is_streaming_request(self, endpoint: str, request_data: JsonObject, /) -> bool: ...
    def validate_environment(self, **arguments: Unpack[EnvironmentArguments]) -> dict[str, str]: ...


CHAT: Final[ChatConfig] = NvidiaNimConfig()
EMBEDDING: Final[EmbeddingConfig] = NvidiaNimEmbeddingConfig()
PASSTHROUGH: Final[PassthroughConfig] = NvidiaNimPassthroughConfig()


class Case(TypedDict, total=False):
    config: Kind
    model: str
    non_default: JsonObject
    optional: JsonObject
    kwargs: JsonObject | None
    top_n: int | None
    query: str
    documents: list[Document]
    api_base: str | None
    env_base: str | None
    endpoint: str
    model_group: str | None
    api_key: str | None
    env_key: str | None
    global_key: str | None
    headers: dict[str, str]
    stream: JsonValue
    status: int
    body: str
    request_data: JsonObject


class Input(TypedDict, total=False):
    kind: Operation
    models: list[str]
    urls: list[Case]
    cases: list[Case]
    headers: list[Case]


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def provider(kind: Kind) -> RerankConfig:
    # A fresh config per call: the ranking config keeps top_n on the instance
    if kind == "ranking":
        return NvidiaNimRankingConfig()
    return NvidiaNimRerankConfig()


def complete_url(model: str, api_base: str | None) -> str:
    config: RerankConfig = get_nvidia_nim_rerank_config(model)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/rerank/common_utils.py#L5
    return config.get_complete_url(api_base, model)


def mapped_params(config: RerankConfig, case: Case) -> JsonObject:
    arguments = MapArguments(
        non_default_params=case["non_default"],
        model=case["model"],
        drop_params=False,
        query=case["query"],
        documents=case["documents"],
        top_n=case["top_n"],
    )
    return config.map_cohere_rerank_params(**arguments)


def chat_supported(input_value: Input) -> JsonValue:
    supported = [CHAT.get_supported_openai_params(model) for model in input_value["models"]]
    return {"supported": JSON.validate_python(supported)}


def chat_map(input_value: Input) -> JsonValue:
    mapped = [
        CHAT.map_openai_params(
            case["non_default"], dict(case["optional"]), case["model"], drop_params=False
        )
        for case in input_value["cases"]
    ]
    return {"mapped": JSON.validate_python(mapped)}


def embedding_map(input_value: Input) -> JsonValue:
    results: list[JsonValue] = []
    for case in input_value["cases"]:
        try:
            mapped = EMBEDDING.map_openai_params(
                case["non_default"], dict(case["optional"]), case["kwargs"]
            )
        except AttributeError:
            results.append({"ok": False})
            continue
        results.append({"ok": True, "mapped": mapped})
    supported = EMBEDDING.get_supported_openai_params()
    return {"supported": JSON.validate_python(supported), "mapped": results}


def rerank_select(input_value: Input) -> JsonValue:
    selected = [complete_url(model, None) for model in input_value["models"]]
    urls = [complete_url(case["model"], case["api_base"]) for case in input_value["urls"]]
    return {"selected": JSON.validate_python(selected), "urls": JSON.validate_python(urls)}


def rerank_params(input_value: Input) -> JsonValue:
    mapped: list[JsonValue] = []
    for case in input_value["cases"]:
        mapped.append(mapped_params(provider(case["config"]), case))
    results: list[JsonValue] = []
    for case in input_value["headers"]:
        set_env("NVIDIA_NIM_API_KEY", case["env_key"])
        litellm.api_key = case["global_key"]
        try:
            headers = provider("retrieval").validate_environment(
                dict(case["headers"]), "meta/llama", case["api_key"]
            )
        except ValueError:
            results.append({"ok": False})
            continue
        results.append({"ok": True, "headers": JSON.validate_python(headers)})
    set_env("NVIDIA_NIM_API_KEY", None)
    litellm.api_key = None
    return {"mapped": mapped, "headers": results}


def rerank_request(input_value: Input) -> JsonValue:
    bodies: list[JsonValue] = []
    for case in input_value["cases"]:
        config = provider(case["config"])
        optional = mapped_params(config, case)
        try:
            body = config.transform_rerank_request(case["model"], optional, {})
        except ValueError:
            bodies.append({"ok": False})
            continue
        bodies.append({"ok": True, "body": body})
    return {"bodies": bodies}


def rerank_response(input_value: Input) -> JsonValue:
    responses: list[JsonValue] = []
    for case in input_value["cases"]:
        arguments = ResponseArguments(
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
            api_key=None,
            request_data=case["request_data"],
            optional_params=case["optional"],
        )
        try:
            response = provider(case["config"]).transform_rerank_response(**arguments)
        except ERRORS:
            responses.append({"ok": False})
            continue
        except BaseLLMException:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/rerank/transformation.py#L303-L308
            responses.append({"ok": False})
            continue
        responses.append(JSON.validate_json(response.model_dump_json()))
    return {"responses": responses}


def passthrough_url(input_value: Input) -> JsonValue:
    urls: list[JsonValue] = []
    for case in input_value["cases"]:
        set_env("NVIDIA_NIM_API_BASE", case["env_base"])
        litellm_params: JsonObject = {}
        if case["model_group"] is not None:
            litellm_params["litellm_metadata"] = {"model_group": case["model_group"]}
        arguments = UrlArguments(
            api_base=case["api_base"],
            api_key=None,
            model=case["model"],
            endpoint=case["endpoint"],
            request_query_params=None,
            litellm_params=litellm_params,
        )
        try:
            url, root = PASSTHROUGH.get_complete_url(**arguments)
        except ValueError:
            urls.append({"ok": False})
            continue
        urls.append({"ok": True, "url": str(url), "root": root})
    set_env("NVIDIA_NIM_API_BASE", None)
    return {"urls": urls}


def passthrough_credentials(input_value: Input) -> JsonValue:
    headers: list[JsonValue] = []
    streaming: list[JsonValue] = []
    for case in input_value["cases"]:
        arguments = EnvironmentArguments(
            headers=dict(case["headers"]),
            model="meta/llama",
            messages=[],
            optional_params={},
            litellm_params={},
            api_key=case["api_key"],
            api_base=None,
        )
        headers.append(JSON.validate_python(PASSTHROUGH.validate_environment(**arguments)))
        streaming.append(
            PASSTHROUGH.is_streaming_request("chat/completions", {"stream": case["stream"]})
        )
    return {"headers": headers, "streaming": streaming}


OPERATIONS: Final[dict[Operation, Callable[[Input], JsonValue]]] = {
    "chat_supported": chat_supported,
    "chat_map": chat_map,
    "embedding_map": embedding_map,
    "rerank_select": rerank_select,
    "rerank_params": rerank_params,
    "rerank_request": rerank_request,
    "rerank_response": rerank_response,
    "passthrough_url": passthrough_url,
    "passthrough_credentials": passthrough_credentials,
}


def run(input_value: Input) -> JsonValue:
    operation = OPERATIONS[input_value["kind"]]
    return operation(input_value)
