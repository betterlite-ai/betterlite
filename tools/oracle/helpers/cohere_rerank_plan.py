import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.cohere.rerank.transformation import CohereRerankConfig
from litellm.llms.cohere.rerank_v2.transformation import CohereRerankV2Config
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue

Params = dict[str, JsonValue]
Document = str | dict[str, JsonValue]
OptText = str | None
OptCount = int | None

MapParams = Callable[
    [
        Params | None,
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
        self, headers: dict[str, str], model: str, api_key: str | None, optional_params: Params
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, model: str, optional_params: Params
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


class KeySettings(TypedDict):
    cohere_api_key: NotRequired[str | None]
    co_api_key: NotRequired[str | None]
    cohere_key: NotRequired[str | None]


class Options(TypedDict):
    query: str
    documents: list[Document]
    top_n: NotRequired[int | None]
    rank_fields: NotRequired[list[str] | None]
    return_documents: NotRequired[bool | None]
    max_chunks_per_doc: NotRequired[int | None]
    max_tokens_per_doc: NotRequired[int | None]
    instruction: NotRequired[str | None]


class Reply(TypedDict):
    status: int
    body: str


class Call(TypedDict):
    version: Literal["v1", "v2"]
    model: str
    headers: dict[str, str]
    settings: KeySettings
    options: Options
    non_default: Params
    responses: list[Reply]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]


class Input(TypedDict):
    calls: list[Call]


V1: RerankConfig = CohereRerankConfig()
V2: RerankConfig = CohereRerankV2Config()


def run(input_value: Input) -> JsonValue:
    logging_object = Logging(
        model="rerank-model",
        messages=None,
        stream=False,
        call_type="rerank",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="rerank-call",
        function_id="rerank-function",
    )
    answers: list[JsonValue] = []
    for call in input_value["calls"]:
        for name, key in (
            ("COHERE_API_KEY", call["settings"].get("cohere_api_key")),
            ("CO_API_KEY", call["settings"].get("co_api_key")),
        ):
            os.environ.pop(name, None)
            if key is not None:
                os.environ[name] = key
        litellm.cohere_key = call["settings"].get("cohere_key")
        config = V1 if call["version"] == "v1" else V2
        options = call["options"]
        drop_params = False
        mapped = config.map_cohere_rerank_params(
            call["non_default"],
            call["model"],
            drop_params,
            options["query"],
            options["documents"],
            None,
            options.get("top_n"),
            options.get("rank_fields"),
            options.get("return_documents") is not False,
            options.get("max_chunks_per_doc"),
            options.get("max_tokens_per_doc"),
            options.get("instruction"),
        )
        headers = config.validate_environment(
            call["headers"], call["model"], call.get("api_key"), mapped
        )
        url = config.get_complete_url(call.get("api_base"), call["model"], mapped)
        body = config.transform_rerank_request(call["model"], mapped, headers)
        for reply in call["responses"]:
            config.transform_rerank_response(
                call["model"],
                httpx.Response(reply["status"], content=reply["body"]),
                RerankResponse(),
                logging_object,
            )
        answers.append({"mapped": mapped, "url": url, "headers": {**headers}, "body": body})
    return answers
