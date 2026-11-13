import os
from collections.abc import Callable
from typing import NotRequired, TypedDict, cast

import litellm
import litellm.batches.main as batches
from litellm.batches.main import create_batch as litellm_create_batch
from litellm.types.utils import LiteLLMBatch
from openai import BadRequestError
from pydantic import JsonValue

from batch_provider_create.views import (
    AzureRecorder,
    BaseHandlerRecorder,
    OpenAiRecorder,
    Recorder,
    VertexRecorder,
    XaiRecorder,
)


def install_all() -> list[Recorder]:
    openai_recorder = OpenAiRecorder()
    azure_recorder = AzureRecorder()
    vertex_recorder = VertexRecorder()
    xai_recorder = XaiRecorder()
    base_recorder = BaseHandlerRecorder()
    batches.openai_batches_instance = openai_recorder
    batches.azure_batches_instance = azure_recorder
    batches.vertex_ai_batches_instance = vertex_recorder
    batches.xai_batches_instance = xai_recorder
    batches.base_llm_http_handler = base_recorder
    return [openai_recorder, azure_recorder, vertex_recorder, xai_recorder, base_recorder]


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
    vertex_credentials: str | dict[str, str]
    custom_endpoint: bool


class CreateCall(TypedDict):
    completion_window: str
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
    organization: str
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


create_batch = cast(
    Callable[..., LiteLLMBatch], litellm_create_batch
)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674


def apply_module(module: Module) -> None:
    litellm.api_base = module.get("api_base")
    litellm.organization = module.get("organization")
    litellm.api_key = module.get("api_key")
    litellm.openai_key = module.get("openai_key")
    litellm.api_version = module.get("api_version")
    litellm.azure_key = module.get("azure_key")
    litellm.vertex_project = module.get("vertex_project")
    litellm.vertex_location = module.get("vertex_location")


def run_step(call: CreateCall, recorders: list[Recorder]) -> JsonValue:
    optional = call.get("optional_params")
    if optional is None:
        optional = {}
    custom_llm_provider = call.get("custom_llm_provider")
    if custom_llm_provider is None:
        custom_llm_provider = "openai"
    try:
        create_batch(
            completion_window=call["completion_window"],
            endpoint=call["endpoint"],
            input_file_id=call["input_file_id"],
            custom_llm_provider=custom_llm_provider,
            metadata=call.get("metadata"),
            extra_headers=call.get("extra_headers"),
            extra_body=call.get("extra_body"),
            output_expires_after=call.get("output_expires_after"),
            model=call.get("model"),
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
    calls = [recorded for recorder in recorders for recorded in recorder.calls]
    return calls[-1] if calls else None


def run(input_value: Input) -> JsonValue:
    os.environ.update(input_value["env"])
    apply_module(input_value["module"])
    recorders = install_all()
    return [run_step(step, recorders) for step in input_value["steps"]]
