from collections.abc import Mapping

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.batches.handler import AzureBatchesAPI
from litellm.llms.base_llm.batches.transformation import BaseBatchesConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.openai import OpenAIBatchesAPI
from litellm.llms.vertex_ai.batches.handler import VertexAIBatchPrediction
from litellm.llms.xai.batches.handler import XAIBatchesHandler
from litellm.types.llms.openai import CreateBatchRequest
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.utils import LiteLLMBatch
from openai import AsyncOpenAI, OpenAI
from pydantic import JsonValue
from typing_extensions import override


class Recorder:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []

    def record(self, branch: str, body: JsonValue, fields: dict[str, JsonValue]) -> LiteLLMBatch:
        call: dict[str, JsonValue] = {"branch": branch, "body": body}
        call.update(fields)
        self.calls.append(call)
        return sample_batch()


def sample_batch() -> LiteLLMBatch:
    return LiteLLMBatch(
        id="batch-oracle",
        completion_window="24h",
        created_at=0,
        endpoint="/v1/chat/completions",
        input_file_id="file-oracle",
        object="batch",
        status="validating",
    )


def pairs(values: Mapping[str, str]) -> list[JsonValue]:
    return [
        [key, value]
        for key, value in sorted(values.items())
    ]


def expiry_json(value: Mapping[str, JsonValue] | None) -> JsonValue:
    if value is None:
        return None
    return {"anchor": value.get("anchor"), "seconds": value.get("seconds")}


def body_json(data: CreateBatchRequest) -> JsonValue:
    metadata = data.get("metadata")
    extra_headers = data.get("extra_headers")
    extra_body = data.get("extra_body")
    return {
        "completion_window": data.get("completion_window"),
        "endpoint": data.get("endpoint"),
        "input_file_id": data.get("input_file_id"),
        "metadata": None if metadata is None else pairs(metadata),
        "extra_headers": None if extra_headers is None else pairs(extra_headers),
        "extra_body": None if extra_body is None else pairs(extra_body),
        "output_expires_after": expiry_json(data.get("output_expires_after")),
    }


def timeout_json(value: float | httpx.Timeout) -> JsonValue:
    match value:
        case float():
            return value
        case httpx.Timeout():
            return None


def credentials_json(value: VERTEX_CREDENTIALS_TYPES | None) -> JsonValue:
    if value is None:
        return None
    match value:
        case str():
            return value
        case dict():
            return pairs(value)


class OpenAiRecorder(Recorder, OpenAIBatchesAPI):
    @override
    def create_batch(
        self,
        _is_async: bool,
        create_batch_data: CreateBatchRequest,
        api_key: str | None,
        api_base: str | None,
        timeout: float | httpx.Timeout,
        max_retries: int | None,
        organization: str | None,
        client: OpenAI | AsyncOpenAI | None = None,
    ) -> LiteLLMBatch:
        return self.record(
            "openai_compatible",
            body_json(create_batch_data),
            {
                "api_base": api_base,
                "api_key": api_key,
                "organization": organization,
                "timeout": timeout_json(timeout),
                "max_retries": max_retries,
                "is_async": _is_async,
            },
        )


class AzureRecorder(Recorder, AzureBatchesAPI):
    @override
    def create_batch(
        self,
        _is_async: bool,
        create_batch_data: CreateBatchRequest,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None,
        timeout: float | httpx.Timeout,
        max_retries: int | None,
        client: OpenAI | AsyncOpenAI | None = None,
        litellm_params: Mapping[str, JsonValue] | None = None,
    ) -> LiteLLMBatch:
        return self.record(
            "azure",
            body_json(create_batch_data),
            {
                "api_base": api_base,
                "api_key": api_key,
                "api_version": api_version,
                "timeout": timeout_json(timeout),
                "max_retries": max_retries,
                "is_async": _is_async,
            },
        )


class VertexRecorder(Recorder, VertexAIBatchPrediction):
    @override
    def create_batch(
        self,
        _is_async: bool,
        create_batch_data: CreateBatchRequest,
        api_base: str | None,
        vertex_credentials: VERTEX_CREDENTIALS_TYPES | None,
        vertex_project: str | None,
        vertex_location: str | None,
        timeout: float | httpx.Timeout,
        max_retries: int | None,
        custom_endpoint: bool | None = None,
    ) -> LiteLLMBatch:
        return self.record(
            "vertex_ai",
            body_json(create_batch_data),
            {
                "api_base": api_base,
                "vertex_project": vertex_project,
                "vertex_location": vertex_location,
                "vertex_credentials": credentials_json(vertex_credentials),
                "timeout": timeout_json(timeout),
                "max_retries": max_retries,
                "custom_endpoint": custom_endpoint,
                "is_async": _is_async,
            },
        )


class XaiRecorder(Recorder, XAIBatchesHandler):
    @override
    def create_batch(
        self,
        _is_async: bool,
        create_batch_data: CreateBatchRequest,
        api_base: str | None,
        api_key: str | None,
        timeout: float | httpx.Timeout,
    ) -> LiteLLMBatch:
        return self.record(
            "xai",
            body_json(create_batch_data),
            {
                "api_base": api_base,
                "api_key": api_key,
                "timeout": timeout_json(timeout),
                "is_async": _is_async,
            },
        )


class BaseHandlerRecorder(Recorder, BaseLLMHTTPHandler):
    @override
    def create_batch(
        self,
        create_batch_data: CreateBatchRequest,
        litellm_params: Mapping[str, JsonValue],
        provider_config: BaseBatchesConfig,
        headers: Mapping[str, str],
        api_base: str | None,
        api_key: str | None,
        logging_obj: Logging,
        _is_async: bool = False,
        client: HTTPHandler | AsyncHTTPHandler | None = None,
        timeout: float | httpx.Timeout | None = None,
        model: str | None = None,
    ) -> LiteLLMBatch:
        return self.record(
            "provider_config",
            body_json(create_batch_data),
            {
                "provider": provider_config.custom_llm_provider.value,
                "model": model,
                "headers": pairs(headers),
                "api_base": api_base,
                "api_key": api_key,
                "timeout": None if timeout is None else timeout_json(timeout),
                "is_async": _is_async,
            },
        )
