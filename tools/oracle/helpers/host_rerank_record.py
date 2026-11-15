import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.infinity.rerank.transformation import InfinityRerankConfig
from litellm.llms.jina_ai.rerank.transformation import JinaAIRerankConfig
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter

Params = dict[str, JsonValue]
Document = str | dict[str, JsonValue]
OptText = str | None
OptCount = int | None
FALLBACK_ID = 0x0000_0000_0000_4000_8000_0000_0000_0001
ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

MapParams = Callable[
    [
        Params,
        str,
        bool,
        str,
        list[Document],
        OptText,
        OptCount,
        list[str] | None,
        bool | None,
        OptCount,
        OptCount,
        OptText,
    ],
    Params,
]


class RerankConfig(Protocol):
    @property
    def map_cohere_rerank_params(self) -> MapParams: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: OptText, optional_params: Params
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: OptText, model: str, optional_params: Params
    ) -> str: ...

    def transform_rerank_request(
        self, model: str, optional_rerank_params: Params, headers: dict[str, str]
    ) -> Params: ...

    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Logging,
    ) -> RerankResponse: ...


class Options(TypedDict):
    query: str
    documents: list[Document]
    top_n: NotRequired[int | None]
    return_documents: NotRequired[bool | None]


class Reply(TypedDict):
    status: int
    body: str


class Input(TypedDict):
    provider: Literal["jina", "infinity"]
    model: str
    options: Options
    non_default: NotRequired[Params]
    headers: NotRequired[dict[str, str]]
    env: NotRequired[dict[str, str]]
    global_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    reply: NotRequired[Reply | None]


class Hook:
    def __init__(self) -> None:
        self.events: list[JsonValue] = []

    def __call__(self, details: dict[str, JsonValue]) -> None:
        self.events.append(details["original_response"])


class RecordingLogging(Logging):
    logger_fn: Hook | None = None


class FixedUuid:
    def __init__(self, value: int) -> None:
        self.value = UUID(int=value)

    def __call__(self) -> UUID:
        return self.value


JINA: RerankConfig = JinaAIRerankConfig()
INFINITY: RerankConfig = InfinityRerankConfig()


def config_for(provider: Literal["jina", "infinity"]) -> RerankConfig:
    if provider == "jina":
        return JINA
    return INFINITY


def apply_environment(env: dict[str, str] | None) -> None:
    os.environ.pop("INFINITY_API_KEY", None)
    if env is None:
        return
    for name, value in env.items():
        os.environ[name] = value


def caller_headers(input_value: Input) -> dict[str, str]:
    headers = input_value.get("headers")
    if headers is None:
        return {}
    return headers


def map_options(config: RerankConfig, input_value: Input) -> Params:
    options = input_value["options"]
    non_default = input_value.get("non_default")
    if non_default is None:
        non_default = {}
    drop_params = False
    return config.map_cohere_rerank_params(
        non_default,
        input_value["model"],
        drop_params,
        options["query"],
        options["documents"],
        None,
        options.get("top_n"),
        None,
        options.get("return_documents") is not False,
        None,
        None,
        None,
    )


def decode_reply(config: RerankConfig, input_value: Input, logging_object: Logging) -> JsonValue:
    reply = input_value.get("reply")
    if reply is None:
        return None
    # an id-less reply must carry the same fixed id as the Rust helper
    litellm_uuid.uuid4 = FixedUuid(FALLBACK_ID)
    transformed = config.transform_rerank_response(
        input_value["model"],
        httpx.Response(reply["status"], content=reply["body"]),
        RerankResponse(),
        logging_object,
    )
    return ADAPTER.validate_json(transformed.model_dump_json())


def run(input_value: Input) -> JsonValue:
    config = config_for(input_value["provider"])
    model = input_value["model"]
    apply_environment(input_value.get("env"))
    litellm.infinity_key = input_value.get("global_key")
    hook = Hook()
    logging_object = RecordingLogging(
        model="rerank-model",
        messages=None,
        stream=False,
        call_type="rerank",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="rerank-call",
        function_id="rerank-function",
    )
    logging_object.logger_fn = hook
    mapped = map_options(config, input_value)
    headers = config.validate_environment(
        caller_headers(input_value), model, input_value.get("api_key"), mapped
    )
    url = config.get_complete_url(input_value.get("api_base"), model, mapped)
    request = config.transform_rerank_request(model, mapped, headers)
    response = decode_reply(config, input_value, logging_object)
    return {
        "mapped": mapped,
        "headers": {**headers},
        "url": url,
        "request": request,
        "response": response,
        "hook": hook.events,
    }
