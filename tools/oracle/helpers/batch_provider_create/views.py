import json
from collections.abc import Mapping
from typing import ClassVar, Final, TypeAlias

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.batches.handler import AzureBatchesAPI
from litellm.llms.base_llm.batches.transformation import BaseBatchesConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.openai import OpenAIBatchesAPI
from litellm.llms.vertex_ai.batches.handler import VertexAIBatchPrediction
from litellm.llms.xai.batches.handler import XAIBatchesHandler
from litellm.types.llms.openai import CreateBatchRequest, FileExpiresAfter
from litellm.types.utils import LiteLLMBatch
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import ConfigDict, JsonValue, TypeAdapter

Recordable: TypeAlias = (
    JsonValue
    | dict[str, str]
    | CreateBatchRequest
    | httpx.Timeout
    | BaseBatchesConfig
    | Logging
    | HTTPHandler
    | AsyncHTTPHandler
    | OpenAI
    | AsyncOpenAI
    | AzureOpenAI
    | AsyncAzureOpenAI
)
FieldValue: TypeAlias = JsonValue | dict[str, str] | httpx.Timeout | BaseBatchesConfig
FIELDS: Final = TypeAdapter(FieldValue, config=ConfigDict(arbitrary_types_allowed=True))

BASE_KEYS: Final = {
    "api_base": "api_base",
    "api_key": "api_key",
    "timeout": "timeout",
    "is_async": "_is_async",
}
OPENAI_KEYS: Final = {**BASE_KEYS, "organization": "organization", "max_retries": "max_retries"}
AZURE_KEYS: Final = {**BASE_KEYS, "api_version": "api_version", "max_retries": "max_retries"}
VERTEX_KEYS: Final = {
    "api_base": "api_base",
    "timeout": "timeout",
    "is_async": "_is_async",
    "vertex_project": "vertex_project",
    "vertex_location": "vertex_location",
    "vertex_credentials": "vertex_credentials",
    "max_retries": "max_retries",
    "custom_endpoint": "custom_endpoint",
}
PROVIDER_KEYS: Final = {
    **BASE_KEYS,
    "provider": "provider_config",
    "model": "model",
    "headers": "headers",
}


class Recording:
    branch: ClassVar[str]
    keys: ClassVar[Mapping[str, str]]

    def create_batch(self, *_args: Recordable, **kwargs: Recordable) -> LiteLLMBatch:
        call: dict[str, JsonValue] = {
            "branch": self.branch,
            "body": body_json(kwargs["create_batch_data"]),
        }
        for output, source in self.keys.items():
            call[output] = field_json(kwargs[source])
        return LiteLLMBatch(
            id="batch-oracle",
            completion_window="24h",
            created_at=0,
            endpoint="/v1/chat/completions",
            input_file_id="file-oracle",
            object="batch",
            status="validating",
            metadata={"call": json.dumps(call)},
        )


class OpenAiRecorder(Recording, OpenAIBatchesAPI):
    branch = "openai_compatible"
    keys: ClassVar[Mapping[str, str]] = OPENAI_KEYS


class AzureRecorder(Recording, AzureBatchesAPI):
    branch = "azure"
    keys: ClassVar[Mapping[str, str]] = AZURE_KEYS


class VertexRecorder(Recording, VertexAIBatchPrediction):
    branch = "vertex_ai"
    keys: ClassVar[Mapping[str, str]] = VERTEX_KEYS


class XaiRecorder(Recording, XAIBatchesHandler):
    branch = "xai"
    keys: ClassVar[Mapping[str, str]] = BASE_KEYS


class ProviderConfigRecorder(Recording, BaseLLMHTTPHandler):
    branch = "provider_config"
    keys: ClassVar[Mapping[str, str]] = PROVIDER_KEYS


def pairs(values: Mapping[str, JsonValue]) -> list[JsonValue]:
    sorted_pairs: list[JsonValue] = []
    for key, value in sorted(values.items()):
        sorted_pairs.append([key, value])
    return sorted_pairs


def field_json(value: Recordable) -> JsonValue:
    match FIELDS.validate_python(value):
        case None | httpx.Timeout():
            return None
        case bool() | int() | float() | str() | list() as plain:
            return plain
        case BaseBatchesConfig() as config:
            return config.custom_llm_provider.value
        case dict() as mapping:
            return pairs(mapping)


def expiry_json(value: FileExpiresAfter | None) -> JsonValue:
    if value is None:
        return None
    return {"anchor": value["anchor"], "seconds": value["seconds"]}


def body_json(data: Recordable) -> JsonValue:
    request = TypeAdapter(CreateBatchRequest).validate_python(data)
    metadata = request.get("metadata")
    extra_headers = request.get("extra_headers")
    extra_body = request.get("extra_body")
    expiry = request.get("output_expires_after")
    return {
        "completion_window": request.get("completion_window"),
        "endpoint": request.get("endpoint"),
        "input_file_id": request.get("input_file_id"),
        "metadata": None if metadata is None else pairs(metadata),
        "extra_headers": None if extra_headers is None else pairs(extra_headers),
        "extra_body": None if extra_body is None else pairs(extra_body),
        "output_expires_after": expiry_json(expiry),
    }
