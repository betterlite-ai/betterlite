import os
from collections.abc import Callable
from datetime import UTC, datetime
from functools import partial
from typing import Final, Protocol, TypeAlias, TypedDict, cast

import httpx
from httpx._types import RequestFiles
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.hosted_vllm.decisions.transformation import HostedVLLMDecisionsConfig
from litellm.llms.hosted_vllm.image_edit.transformation import HostedVLLMImageEditConfig
from litellm.llms.hosted_vllm.rerank.transformation import HostedVLLMRerankConfig
from litellm.llms.hosted_vllm.responses.transformation import HostedVLLMResponsesAPIConfig
from litellm.llms.hosted_vllm.transcriptions.transformation import (
    HostedVLLMAudioTranscriptionConfig,
)
from litellm.llms.hosted_vllm.videos.transformation import HostedVLLMVideoConfig
from litellm.types.rerank import RerankResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Object: TypeAlias = dict[str, JsonValue]


class RerankConfig(Protocol):
    def get_complete_url(self, *, api_base: str | None, model: str) -> str: ...

    # ast-grep-ignore: no-long-names
    def get_supported_cohere_rerank_params(self, *, model: str) -> list[str]: ...

    def map_cohere_rerank_params(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/hosted_vllm/rerank/transformation.py#L83-L97
        self, *, non_default_params: Object, model: str, drop_params: bool, query: str,
        documents: list[str | Object], top_n: int | None = None,
        rank_fields: list[str] | None = None,
        return_documents: bool | None = True, max_chunks_per_doc: int | None = None,
        max_tokens_per_doc: int | None = None, instruction: str | None = None,
    ) -> Object: ...

    def validate_environment(
        self, *, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...

    def transform_rerank_request(
        self, *, model: str, optional_rerank_params: Object, headers: dict[str, str]
    ) -> Object: ...

    def transform_rerank_response(
        self, *, model: str, raw_response: httpx.Response, model_response: RerankResponse,
        logging_obj: Logging,
    ) -> RerankResponse: ...


class ResponsesConfig(Protocol):
    def get_complete_url(self, *, api_base: str | None, litellm_params: Object) -> str: ...

    def validate_environment(
        self, *, headers: dict[str, str], model: str, litellm_params: GenericLiteLLMParams
    ) -> dict[str, str]: ...

    def supports_native_websocket(self) -> bool: ...


class ImageEditConfig(Protocol):
    def get_supported_openai_params(self, *, model: str) -> list[str]: ...

    def validate_environment(
        self, *, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, *, model: str, api_base: str | None, litellm_params: Object
    ) -> str: ...


class TranscriptionData(Protocol):
    data: bytes | Object


class TranscriptionConfig(Protocol):
    def get_complete_url(
        self, *, api_base: str | None, api_key: str | None, model: str,
        optional_params: Object, litellm_params: Object,
    ) -> str: ...

    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self, *, model: str, audio_file: str, optional_params: Object, litellm_params: Object
    ) -> TranscriptionData: ...


class VideoConfig(Protocol):
    def get_supported_openai_params(self, *, model: str) -> list[str]: ...

    def validate_environment(
        self, *, headers: dict[str, str], model: str, api_key: str | None = None,
        litellm_params: GenericLiteLLMParams | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, *, model: str, api_base: str | None, litellm_params: Object
    ) -> str: ...

    def transform_video_create_request(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/hosted_vllm/videos/transformation.py#L149-L156
        self, *, model: str, prompt: str, api_base: str,
        video_create_optional_request_params: Object,  # ast-grep-ignore: no-long-names
        litellm_params: GenericLiteLLMParams, headers: dict[str, str],
    ) -> tuple[Object, RequestFiles, str]: ...


class DecisionsConfig(Protocol):
    api_key_env: tuple[str, ...]
    api_base_env: tuple[str, ...]
    api_key_required: bool

    def resolve_api_key(self, api_key: str | None) -> str | None: ...


# a call carries the keys its function reads, so one shape serves every dispatch
class Call(TypedDict):
    api_base: str | None
    api_key: str | None
    audio: str
    body: str
    documents: list[str | Object]
    env: dict[str, str]
    headers: dict[str, str]
    instruction: str | None
    mapped: Object
    max_chunks_per_doc: int | None
    max_tokens_per_doc: int | None
    model: str
    non_default_params: Object
    options: Object
    params: Object
    params_api_key: str | None
    prompt: str
    query: str
    rank_fields: list[str] | None
    return_documents: bool | None
    top_n: int | None


class Input(TypedDict):
    function: str
    calls: list[Call]


def replace_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(environment)


def rerank_value(function: str, call: Call) -> JsonValue:
    if function == "rerank_params":
        replace_environment(call["env"])
    client = cast("RerankConfig", HostedVLLMRerankConfig())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/hosted_vllm/rerank/transformation.py#L48
    if function == "rerank_request":
        body = client.transform_rerank_request(
            model=call["model"], optional_rerank_params=call["mapped"], headers={}
        )
        return JSON.validate_python(body)
    if function == "rerank_reply":
        raw_response = httpx.Response(
            200, content=call["body"].encode(), request=httpx.Request("POST", "https://host")
        )
        messages: list[dict[str, JsonValue]] = []
        reply = client.transform_rerank_response(
            model=call["model"], raw_response=raw_response, model_response=RerankResponse(),
            logging_obj=Logging(
                model=call["model"], messages=messages, stream=False,
                call_type=CallTypes.rerank, start_time=datetime(2026, 1, 1, tzinfo=UTC),
                litellm_call_id="b8151", function_id="b8151",
            ),
        )
        return JSON.validate_json(reply.model_dump_json())
    answer: Object = {
        "supported": JSON.validate_python(
            client.get_supported_cohere_rerank_params(model=call["model"])
        ),
        "url": client.get_complete_url(api_base=call["api_base"], model=call["model"]),
        "headers": JSON.validate_python(
            client.validate_environment(
                headers=call["headers"], model=call["model"], api_key=call["api_key"]
            )
        ),
        "mapped": client.map_cohere_rerank_params(
            non_default_params=call["non_default_params"], model=call["model"],
            drop_params=False, query=call["query"], documents=call["documents"],
            top_n=call["top_n"], rank_fields=call["rank_fields"],
            return_documents=call["return_documents"],
            max_chunks_per_doc=call["max_chunks_per_doc"],
            max_tokens_per_doc=call["max_tokens_per_doc"], instruction=call["instruction"],
        ),
    }
    return JSON.validate_python(answer)


def media_value(function: str, call: Call) -> JsonValue:
    if function == "transcription_request":
        client = cast("TranscriptionConfig", HostedVLLMAudioTranscriptionConfig())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/hosted_vllm/transcriptions/transformation.py#L29
        url = client.get_complete_url(
            api_base=call["api_base"], api_key=None, model=call["model"],
            optional_params={}, litellm_params={},
        )
        request = client.transform_audio_transcription_request(
            model=call["model"], audio_file=call["audio"], optional_params=call["options"],
            litellm_params={},
        )
        request_answer: Object = {"url": url, "data": JSON.validate_python(request.data)}
        return JSON.validate_python(request_answer)
    replace_environment(call["env"])
    if function == "image_edit_url":
        image_client = cast("ImageEditConfig", HostedVLLMImageEditConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py#L30
        image_url = image_client.get_complete_url(
            model=call["model"], api_base=call["api_base"], litellm_params={}
        )
        image_headers = image_client.validate_environment(
            headers=call["headers"], model=call["model"], api_key=call["api_key"]
        )
        supported = image_client.get_supported_openai_params(model=call["model"])
        image_answer: Object = {
            "supported": JSON.validate_python(supported),
            "url": image_url,
            "headers": JSON.validate_python(image_headers),
        }
        return JSON.validate_python(image_answer)
    if function == "responses_url":
        responses_client = cast("ResponsesConfig", HostedVLLMResponsesAPIConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L31
        params = GenericLiteLLMParams(api_key=call["api_key"])
        responses_url = responses_client.get_complete_url(
            api_base=call["api_base"], litellm_params={}
        )
        responses_headers = responses_client.validate_environment(
            headers=call["headers"], model=call["model"], litellm_params=params
        )
        websocket = responses_client.supports_native_websocket()
        responses_answer: Object = {
            "url": responses_url,
            "headers": JSON.validate_python(responses_headers),
            "websocket": websocket,
        }
        return JSON.validate_python(responses_answer)
    video_client = cast("VideoConfig", HostedVLLMVideoConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/videos/transformation.py#L47
    video_params = GenericLiteLLMParams(api_key=call["params_api_key"])
    video_url = video_client.get_complete_url(
        model=call["model"], api_base=call["api_base"], litellm_params={}
    )
    video_headers = video_client.validate_environment(
        headers=call["headers"], model=call["model"], api_key=call["api_key"],
        litellm_params=video_params,
    )
    data, _files, _base = video_client.transform_video_create_request(
        model=call["model"], prompt=call["prompt"], api_base=video_url,
        video_create_optional_request_params=call["params"], litellm_params=video_params,
        headers=video_headers,
    )
    video_supported = video_client.get_supported_openai_params(model=call["model"])
    video_answer: Object = {
        "supported": JSON.validate_python(video_supported),
        "url": video_url,
        "headers": JSON.validate_python(video_headers),
        "fields": data,
    }
    return JSON.validate_python(video_answer)


def decisions_value(call: Call) -> JsonValue:
    replace_environment(call["env"])
    config = cast("DecisionsConfig", HostedVLLMDecisionsConfig())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/hosted_vllm/decisions/transformation.py#L8
    answer: Object = {
        "api_key_env": [*config.api_key_env],
        "api_base_env": [*config.api_base_env],
        "api_key_required": config.api_key_required,
        "resolved": config.resolve_api_key(call["api_key"]),
    }
    return JSON.validate_python(answer)


VALUES: Final[dict[str, Callable[[Call], JsonValue]]] = {
    "rerank_params": partial(rerank_value, "rerank_params"),
    "rerank_request": partial(rerank_value, "rerank_request"),
    "rerank_reply": partial(rerank_value, "rerank_reply"),
    "responses_url": partial(media_value, "responses_url"),
    "image_edit_url": partial(media_value, "image_edit_url"),
    "transcription_request": partial(media_value, "transcription_request"),
    "video_form": partial(media_value, "video_form"),
    "decisions_key": decisions_value,
}


def run(input_value: Input) -> list[JsonValue]:
    build = VALUES[input_value["function"]]
    return [build(call) for call in input_value["calls"]]
