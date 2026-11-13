from collections.abc import Mapping
from typing import Final

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.batches.transformation import BaseBatchesConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.openai.openai import OpenAIBatchesAPI
from litellm.types.llms.openai import CreateBatchRequest, FileExpiresAfter
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.utils import LiteLLMBatch
from openai import AsyncOpenAI, OpenAI
from pydantic import JsonValue
from typing_extensions import override

BASE_KEYS: Final = ("api_base", "api_key", "timeout", "is_async")
OPENAI_KEYS: Final = (*BASE_KEYS, "organization", "max_retries")
AZURE_KEYS: Final = (*BASE_KEYS, "api_version", "max_retries")
VERTEX_KEYS: Final = (
    "api_base",
    "timeout",
    "is_async",
    "vertex_project",
    "vertex_location",
    "vertex_credentials",
    "max_retries",
    "custom_endpoint",
)
PROVIDER_KEYS: Final = (*BASE_KEYS, "provider", "model", "headers")


class Recorder(OpenAIBatchesAPI):
    def __init__(self, branch: str, keys: tuple[str, ...]) -> None:
        super().__init__()
        self.branch = branch
        self.keys = keys
        self.calls: list[JsonValue] = []

    @override
    def create_batch(
        self,
        _is_async: bool,
        create_batch_data: CreateBatchRequest,
        api_key: str | None = None,
        api_base: str | None = None,
        timeout: float | httpx.Timeout | None = None,
        max_retries: int | None = None,
        organization: str | None = None,
        client: OpenAI | AsyncOpenAI | HTTPHandler | AsyncHTTPHandler | None = None,
        api_version: str | None = None,
        litellm_params: Mapping[str, JsonValue] | None = None,
        provider_config: BaseBatchesConfig | None = None,
        headers: Mapping[str, str] | None = None,
        logging_obj: Logging | None = None,
        vertex_project: str | None = None,
        vertex_location: str | None = None,
        vertex_credentials: VERTEX_CREDENTIALS_TYPES | None = None,
        custom_endpoint: bool | None = None,
        model: str | None = None,
    ) -> LiteLLMBatch:
        provider = None if provider_config is None else provider_config.custom_llm_provider.value
        available: dict[str, JsonValue] = {
            "api_base": api_base,
            "api_key": api_key,
            "timeout": timeout_json(timeout),
            "is_async": _is_async,
            "organization": organization,
            "max_retries": max_retries,
            "api_version": api_version,
            "vertex_project": vertex_project,
            "vertex_location": vertex_location,
            "vertex_credentials": credentials_json(vertex_credentials),
            "custom_endpoint": custom_endpoint,
            "provider": provider,
            "model": model,
            "headers": None if headers is None else pairs(headers),
        }
        call: dict[str, JsonValue] = {"branch": self.branch, "body": body_json(create_batch_data)}
        for key in self.keys:
            call[key] = available[key]
        self.calls.append(call)
        return sample_batch()


def recorders() -> list[tuple[str, Recorder]]:
    return [
        ("openai_batches_instance", Recorder("openai_compatible", OPENAI_KEYS)),
        ("azure_batches_instance", Recorder("azure", AZURE_KEYS)),
        ("vertex_ai_batches_instance", Recorder("vertex_ai", VERTEX_KEYS)),
        ("xai_batches_instance", Recorder("xai", BASE_KEYS)),
        ("base_llm_http_handler", Recorder("provider_config", PROVIDER_KEYS)),
    ]


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
    return [[key, value] for key, value in sorted(values.items())]


def expiry_json(value: FileExpiresAfter | None) -> JsonValue:
    if value is None:
        return None
    return {"anchor": value["anchor"], "seconds": value["seconds"]}


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


def timeout_json(value: float | httpx.Timeout | None) -> JsonValue:
    match value:
        case None | httpx.Timeout():
            return None
        case float() | int():
            return value


def credentials_json(value: VERTEX_CREDENTIALS_TYPES | None) -> JsonValue:
    match value:
        case None:
            return None
        case str():
            return value
        case dict():
            return pairs(value)
