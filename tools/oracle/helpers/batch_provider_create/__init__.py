import os
from typing import Literal, NotRequired, Protocol, TypedDict, cast

import litellm
import litellm.batches.main as batches
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.utils import LiteLLMBatch
from openai import BadRequestError
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from batch_provider_create.views import (
    AzureRecorder,
    OpenAiRecorder,
    ProviderConfigRecorder,
    VertexRecorder,
    XaiRecorder,
)


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
