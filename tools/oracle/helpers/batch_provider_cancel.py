import asyncio
import os
from collections.abc import Callable
from contextlib import ExitStack
from typing import Final, Literal, TypedDict
from unittest.mock import patch

import litellm
from litellm.batches import main as batch_main
from litellm.llms.bedrock.embed.embedding import BedrockEmbedding
from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue, TypeAdapter

WIRE: Final = TypeAdapter(JsonValue)

Provider = Literal["openai", "azure", "vertex_ai", "bedrock", "litellm_proxy", "xai"]

CANCEL, ASYNC_STATUS = litellm.acancel_batch, batch_main._handle_async_invoke_status  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L888 https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L1087


class Settings(TypedDict):
    api_base: str | None
    api_version: str | None
    api_key: str | None
    openai_key: str | None
    azure_key: str | None
    organization: str | None
    vertex_project: str | None
    vertex_location: str | None


class Params(TypedDict):
    api_base: str | None
    api_key: str | None
    api_version: str | None
    organization: str | None
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | None
    azure_ad_token: str | None
    aws_region_name: str | None
    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    timeout: float | None
    request_timeout: float | None
    max_retries: int | None


class CancelCall(TypedDict):
    batch_id: str
    model: str | None
    provider: Provider
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    params: Params
    call: dict[str, JsonValue]


class S3Output(TypedDict):
    s3Uri: str


class OutputDataConfig(TypedDict):
    s3OutputDataConfig: S3Output


class Status(TypedDict):
    invocationArn: str
    modelArn: str
    status: str
    failureMessage: str | None
    outputDataConfig: OutputDataConfig | None
    submitTime: str | None
    lastModifiedTime: str | None
    endTime: str | None


class CancelEnvelope(TypedDict):
    cancel: CancelCall


class AsyncEnvelope(TypedDict):
    async_invoke: Status


class Script(TypedDict):
    batch: dict[str, JsonValue] | None
    error: str | None


class Input(TypedDict):
    input: CancelEnvelope | AsyncEnvelope
    settings: Settings | None
    env: dict[str, str]
    now: int
    script: Script


class Recorder:
    def __init__(
        self,
        build: Callable[[dict[str, JsonValue]], JsonValue],
        answer: LiteLLMBatch,
        calls: list[JsonValue],
    ) -> None:
        self.build = build
        self.answer = answer
        self.calls = calls

    def cancel_batch(self, **kwargs: JsonValue) -> LiteLLMBatch:
        self.calls.append(self.build(kwargs))
        return self.answer


def openai_record(kwargs: dict[str, JsonValue]) -> JsonValue:
    return WIRE.validate_python(
        {
            "branch": "openai",
            "is_async": kwargs["_is_async"],
            "request": kwargs["cancel_batch_data"],
            "api_base": kwargs["api_base"],
            "api_key": kwargs["api_key"],
            "organization": kwargs["organization"],
            "timeout": kwargs["timeout"],
            "max_retries": kwargs["max_retries"],
        }
    )


def azure_record(kwargs: dict[str, JsonValue]) -> JsonValue:
    return WIRE.validate_python(
        {
            "branch": "azure",
            "is_async": kwargs["_is_async"],
            "request": kwargs["cancel_batch_data"],
            "api_base": kwargs["api_base"],
            "api_key": kwargs["api_key"],
            "api_version": kwargs["api_version"],
            "timeout": kwargs["timeout"],
            "max_retries": kwargs["max_retries"],
        }
    )


def vertex_record(kwargs: dict[str, JsonValue]) -> JsonValue:
    return WIRE.validate_python(
        {
            "branch": "vertex",
            "is_async": kwargs["_is_async"],
            "batch_id": kwargs["batch_id"],
            "api_base": kwargs["api_base"],
            "vertex_project": kwargs["vertex_project"],
            "vertex_location": kwargs["vertex_location"],
            "vertex_credentials": kwargs["vertex_credentials"],
            "timeout": kwargs["timeout"],
            "max_retries": kwargs["max_retries"],
        }
    )


def xai_record(kwargs: dict[str, JsonValue]) -> JsonValue:
    return WIRE.validate_python(
        {
            "branch": "xai",
            "is_async": kwargs["_is_async"],
            "batch_id": kwargs["batch_id"],
            "api_base": kwargs["api_base"],
            "api_key": kwargs["api_key"],
            "timeout": kwargs["timeout"],
        }
    )


def patch_settings(stack: ExitStack, settings: Settings | None) -> None:
    if settings is None:
        return
    stack.enter_context(patch.object(litellm, "api_base", settings["api_base"]))
    stack.enter_context(patch.object(litellm, "api_version", settings["api_version"]))
    stack.enter_context(patch.object(litellm, "api_key", settings["api_key"]))
    stack.enter_context(patch.object(litellm, "openai_key", settings["openai_key"]))
    stack.enter_context(patch.object(litellm, "azure_key", settings["azure_key"]))
    stack.enter_context(patch.object(litellm, "organization", settings["organization"]))
    stack.enter_context(
        patch.object(litellm, "vertex_project", settings["vertex_project"])
    )
    stack.enter_context(
        patch.object(litellm, "vertex_location", settings["vertex_location"])
    )


def cancel(
    call: CancelCall, settings: Settings | None, script: Script, env: dict[str, str]
) -> JsonValue:
    if "async" not in call["call"]:
        raise ValueError("the batch provider cancel helper drives the async entry point")
    answer = LiteLLMBatch.model_validate(script["batch"])
    calls: list[JsonValue] = []
    with ExitStack() as stack:
        stack.enter_context(patch.dict(os.environ, env, clear=False))
        patch_settings(stack, settings)
        openai_instance = Recorder(openai_record, answer, calls)
        azure_instance = Recorder(azure_record, answer, calls)
        vertex_instance = Recorder(vertex_record, answer, calls)
        xai_instance = Recorder(xai_record, answer, calls)
        stack.enter_context(
            patch.object(batch_main, "openai_batches_instance", openai_instance)
        )
        stack.enter_context(
            patch.object(batch_main, "azure_batches_instance", azure_instance)
        )
        stack.enter_context(
            patch.object(batch_main, "vertex_ai_batches_instance", vertex_instance)
        )
        stack.enter_context(
            patch.object(batch_main, "xai_batches_instance", xai_instance)
        )
        batch = asyncio.run(
            CANCEL(
                batch_id=call["batch_id"],
                model=call["model"],
                custom_llm_provider=call["provider"],
                extra_headers=call["extra_headers"],
                extra_body=call["extra_body"],
                **call["params"],
            )
        )
    return WIRE.validate_python(
        {
            "batch": WIRE.validate_json(batch.model_dump_json()),
            "model": call["model"],
            "calls": calls,
        }
    )


class ScriptedStatus:
    def __init__(self, status: Status) -> None:
        self.status = status

    async def __call__(
        self,
        invocation_arn: str,
        aws_region_name: str,
        logging_obj: JsonValue = None,
        **_kwargs: JsonValue,
    ) -> Status:
        del invocation_arn, aws_region_name, logging_obj, _kwargs
        return self.status


def async_invoke(status: Status) -> JsonValue:
    scripted = ScriptedStatus(status)
    with patch.object(BedrockEmbedding, "_get_async_invoke_status", scripted):
        batch = ASYNC_STATUS(
            batch_id=status["invocationArn"], aws_region_name="us-east-1"
        )
    return WIRE.validate_python(
        {"batch": WIRE.validate_json(batch.model_dump_json()), "model": None, "calls": []}
    )


def run(input_value: Input) -> JsonValue:
    envelope = input_value["input"]
    if "cancel" in envelope:
        return cancel(
            envelope["cancel"],
            input_value["settings"],
            input_value["script"],
            input_value["env"],
        )
    return async_invoke(envelope["async_invoke"])
