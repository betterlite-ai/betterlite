import asyncio
from collections.abc import AsyncIterator
from typing import cast

import litellm
import litellm.utils
from litellm.llms.azure_ai.agents.handler import AzureAIAgentsHandler as AgentHandler
from litellm.llms.azure_ai.agents.transformation import AzureAIAgentsError
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue

from .fakes import FakeClient, Recorder, install_client, patch_identity
from .types import (
    JSON,
    REPLY_JSON,
    AnnotationCall,
    ExtractionCall,
    JsonObject,
    ReplyCall,
    RequestCall,
    StreamCall,
    StreamPlan,
)
from .views import chunk_view, fields_of, usage_of, usage_view


def extraction_answer(handler: AgentHandler, call: ExtractionCall) -> JsonValue:
    content, annotations = handler._extract_content_from_messages(call["messages"])  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L158-L175
    return {"content": content, "annotations": JSON.validate_python(annotations)}


def annotation_answer(handler: AgentHandler, call: AnnotationCall) -> JsonValue:
    transformed = handler._transform_annotations(call["annotations"])  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L177-L210
    return JSON.validate_python(transformed)


def reply_answer(handler: AgentHandler, call: ReplyCall) -> JsonValue:
    response = ModelResponse()
    existing = call["existing_usage"]
    if existing is not None:
        response.usage = usage_of(existing)
    hidden = call["hidden_params"]
    if hidden is not None:
        response._hidden_params = hidden  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L226-L230
    recorder = Recorder(fail=call["fail"])
    litellm.utils.token_counter = recorder.count  # noqa: E501  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L231-L232
    built = handler._build_model_response(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L212-L256
        model=call["model"],
        content=call["content"],
        model_response=response,
        thread_id=call["thread_id"],
        messages=cast("list[dict[str, object]]", call["messages"]),  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L212-L256
        annotations=call["annotations"],
    )
    dumped = REPLY_JSON.validate_json(built.model_dump_json())
    first = dumped["choices"][0]
    return {
        "reply": {
            "model": dumped.get("model"),
            "choices": [
                {
                    "index": first.get("index"),
                    "finish_reason": first.get("finish_reason"),
                    "message": fields_of(first["message"]),
                }
            ],
            "hidden_params": cast(JsonObject, built._hidden_params),  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L226-L230
            "usage": usage_view(dumped.get("usage")),
        },
        "counts": JSON.validate_python(recorder.calls),
    }


async def opened_stream(
    handler: AgentHandler, plan: StreamPlan
) -> AsyncIterator[ModelResponseStream]:
    stream = handler.acompletion_stream(
        model=plan["model"],
        messages=cast("list[dict[str, object]]", plan["messages"]),  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L559-L570
        api_base=plan["api_base"],
        api_key=plan["api_key"],
        logging_obj=None,  # noqa: E501  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L563
        optional_params=plan["optional_params"],
        litellm_params=plan["litellm_params"],
        timeout=60.0,
        headers=plan["headers"],
    )
    async for chunk in stream:
        yield chunk


def stream_plan(call: StreamCall) -> StreamPlan:
    return {
        "model": call["model"],
        "messages": [],
        "api_base": "https://base.example/api/projects/p",
        "api_key": "fixture-key",
        "optional_params": {},
        "litellm_params": {},
        "headers": {},
    }


async def collect_chunks(
    handler: AgentHandler, call: StreamCall
) -> list[ModelResponseStream]:
    chunks: list[ModelResponseStream] = []
    async for chunk in opened_stream(handler, stream_plan(call)):
        chunks.append(chunk)
    return chunks


def stream_answer(handler: AgentHandler, call: StreamCall) -> JsonValue:
    patch_identity(call["id"], call["created"])
    install_client(FakeClient(200, call["body"], call["split"]))
    chunks = asyncio.run(collect_chunks(handler, call))
    return {"chunks": [chunk_view(chunk) for chunk in chunks]}


async def request_outcome(handler: AgentHandler, call: RequestCall) -> JsonValue:
    chunks: list[JsonValue] = []
    plan: StreamPlan = {
        "model": call["model"],
        "messages": call["messages"],
        "api_base": call["api_base"],
        "api_key": call["api_key"],
        "optional_params": call["optional_params"],
        "litellm_params": call["litellm_params"],
        "headers": call["headers"],
    }
    try:
        async for chunk in opened_stream(handler, plan):
            chunks.append(chunk_view(chunk))
    except AzureAIAgentsError as error:
        return {"ok": False, "status": error.status_code, "message": str(error)}
    return {"ok": True, "chunks": chunks}


def request_answer(handler: AgentHandler, call: RequestCall) -> JsonValue:
    patch_identity(call["id"], call["created"])
    client = FakeClient(call["status"], call["body"])
    factory = install_client(client)
    outcome = asyncio.run(request_outcome(handler, call))
    return {
        "url": client.url,
        "headers": client.headers,
        "body": client.payload,
        "ssl_verify": factory.ssl_verify,
        "outcome": outcome,
    }
