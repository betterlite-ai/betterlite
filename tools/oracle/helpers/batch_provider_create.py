import json
import os
from collections.abc import Mapping
from typing import ClassVar, Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
import litellm.batches.main as batches
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.batches.handler import AzureBatchesAPI
from litellm.llms.base_llm.batches.transformation import BaseBatchesConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.openai import OpenAIBatchesAPI
from litellm.llms.vertex_ai.batches.handler import VertexAIBatchPrediction
from litellm.llms.xai.batches.handler import XAIBatchesHandler
from litellm.types.llms.openai import CreateBatchRequest, FileExpiresAfter
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.utils import LiteLLMBatch
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, BadRequestError, OpenAI
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override


class OptionalParams(TypedDict, total=False):
    api_base: str
    api_key: str
    api_version: str
    organization: str
    timeout: float
    request_timeout: float
    max_retries: int
    vertex_project: str
    vertex_location: str
    vertex_credentials: VERTEX_CREDENTIALS_TYPES
    custom_endpoint: bool


class CreateCall(TypedDict):
    completion_window: Literal["24h"]
    endpoint: str
    input_file_id: str
    custom_llm_provider: NotRequired[str]
    metadata: NotRequired[dict[str, str] | None]
    extra_headers: NotRequired[dict[str, str] | None]
    extra_body: NotRequired[dict[str, str] | None]
    output_expires_after: NotRequired[dict[str, JsonValue] | None]
    model: NotRequired[str | None]
    optional_params: NotRequired[OptionalParams]


class Module(TypedDict, total=False):
    api_base: str
    api_key: str
    openai_key: str
    api_version: str
    azure_key: str
    vertex_project: str
    vertex_location: str


class Input(TypedDict):
    env: dict[str, str]
    module: Module
    steps: list[CreateCall]


class UnrecordedCallError(Exception):
    pass


class BatchBase(Protocol):
    def __call__(self) -> LiteLLMBatch: ...


class BatchCaller(BatchBase, Protocol):
    @override
    def __call__(
        self,
        *,
        completion_window: Literal["24h"] = ...,
        endpoint: str = ...,
        input_file_id: str = ...,
        custom_llm_provider: str = ...,
        metadata: dict[str, str] | None = ...,
        extra_headers: dict[str, str] | None = ...,
        extra_body: dict[str, str] | None = ...,
        output_expires_after: dict[str, JsonValue] | None = ...,
        model: str | None = ...,
        api_base: str | None = ...,
        api_key: str | None = ...,
        api_version: str | None = ...,
        organization: str | None = ...,
        timeout: float | None = ...,
        request_timeout: float | None = ...,
        max_retries: int | None = ...,
        vertex_project: str | None = ...,
        vertex_location: str | None = ...,
        vertex_credentials: VERTEX_CREDENTIALS_TYPES | None = ...,
        custom_endpoint: bool | None = ...,
    ) -> LiteLLMBatch: ...


create_batch = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674
    BatchCaller,
    batches.create_batch,
)

Recordable: TypeAlias = (
    JsonValue
    | Mapping[str, JsonValue]
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
    ordered = sorted(values.items())
    return [[key, value] for key, value in ordered]


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


def run_step(step: CreateCall) -> JsonValue:
    optional = step.get("optional_params")
    if optional is None:
        optional = OptionalParams()
    provider = step.get("custom_llm_provider")
    if provider is None:
        provider = "openai"
    try:
        batch = create_batch(
            completion_window=step["completion_window"],
            endpoint=step["endpoint"],
            input_file_id=step["input_file_id"],
            custom_llm_provider=provider,
            metadata=step.get("metadata"),
            extra_headers=step.get("extra_headers"),
            extra_body=step.get("extra_body"),
            output_expires_after=step.get("output_expires_after"),
            model=step.get("model"),
            api_base=optional.get("api_base"),
            api_key=optional.get("api_key"),
            api_version=optional.get("api_version"),
            organization=optional.get("organization"),
            timeout=optional.get("timeout"),
            request_timeout=optional.get("request_timeout"),
            max_retries=optional.get("max_retries"),
            vertex_project=optional.get("vertex_project"),
            vertex_location=optional.get("vertex_location"),
            vertex_credentials=optional.get("vertex_credentials"),
            custom_endpoint=optional.get("custom_endpoint"),
        )
    except BadRequestError as error:
        return {"raised": error.status_code}
    if batch.metadata is None:
        raise UnrecordedCallError
    return TypeAdapter(JsonValue).validate_json(batch.metadata["call"])


def run(input_value: Input) -> JsonValue:
    os.environ.update(input_value["env"])
    module = input_value["module"]
    if "api_base" in module:
        litellm.api_base = module["api_base"]
    if "api_key" in module:
        litellm.api_key = module["api_key"]
    if "openai_key" in module:
        litellm.openai_key = module["openai_key"]
    if "api_version" in module:
        litellm.api_version = module["api_version"]
    if "azure_key" in module:
        litellm.azure_key = module["azure_key"]
    if "vertex_project" in module:
        litellm.vertex_project = module["vertex_project"]
    if "vertex_location" in module:
        litellm.vertex_location = module["vertex_location"]
    batches.openai_batches_instance.__class__ = OpenAiRecorder
    batches.azure_batches_instance.__class__ = AzureRecorder
    batches.vertex_ai_batches_instance.__class__ = VertexRecorder
    batches.xai_batches_instance.__class__ = XaiRecorder
    batches.base_llm_http_handler.__class__ = ProviderConfigRecorder
    return [run_step(step) for step in input_value["steps"]]
