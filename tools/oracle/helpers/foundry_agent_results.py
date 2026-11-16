import asyncio
import time
import uuid
from collections.abc import AsyncIterator
from typing import Final, TypeAlias, TypedDict, cast
from uuid import UUID

import httpx
import litellm
import litellm.llms.azure_ai.agents.handler
import litellm.llms.custom_httpx.http_handler
import litellm.utils
from litellm.llms.azure_ai.agents.handler import (
    AzureAIAgentsHandler as AgentHandler,
    _AzureMessagesData,
    _AzureRawAnnotation,
)
from litellm.llms.azure_ai.agents.transformation import AzureAIAgentsError
from litellm.types.llms.openai import ChatCompletionAnnotation
from litellm.types.utils import ModelResponse, ModelResponseStream, Usage
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
JSON: Final = TypeAdapter(JsonValue)
EMPTY: Final[JsonObject] = {}


class ExtractionCall(TypedDict):
    messages: _AzureMessagesData


class AnnotationCall(TypedDict):
    annotations: list[_AzureRawAnnotation]


class ExistingUsage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyCall(TypedDict):
    model: str
    content: str
    thread_id: str
    messages: list[dict[str, object]]
    annotations: list[ChatCompletionAnnotation] | None
    existing_usage: ExistingUsage | None
    hidden_params: JsonObject | None
    fail: bool


class StreamCall(TypedDict):
    model: str
    body: str
    id: str
    created: int
    split: int | None


class RequestCall(TypedDict):
    model: str
    messages: list[dict[str, object]]
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]
    api_base: str
    api_key: str
    status: int
    body: str
    id: str
    created: int


class StreamPlan(TypedDict):
    model: str
    messages: list[dict[str, object]]
    api_base: str
    api_key: str
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]


class Input(TypedDict, total=False):
    extractions: list[ExtractionCall]
    annotations: list[AnnotationCall]
    replies: list[ReplyCall]
    streams: list[StreamCall]
    requests: list[RequestCall]


class DeltaJson(TypedDict, total=False):
    content: JsonValue
    role: JsonValue
    annotations: JsonValue


class StreamingChoiceJson(TypedDict, total=False):
    index: JsonValue
    finish_reason: JsonValue
    delta: DeltaJson


class ChunkJson(TypedDict, total=False):
    id: JsonValue
    created: JsonValue
    model: JsonValue
    object: JsonValue
    choices: list[StreamingChoiceJson]


class MessageJson(TypedDict, total=False):
    role: JsonValue
    content: JsonValue
    annotations: JsonValue


class ReplyChoiceJson(TypedDict, total=False):
    index: JsonValue
    finish_reason: JsonValue
    message: MessageJson


class ReplyJson(TypedDict, total=False):
    model: JsonValue
    choices: list[ReplyChoiceJson]
    usage: JsonObject


CHUNK_JSON: Final = TypeAdapter(ChunkJson)
REPLY_JSON: Final = TypeAdapter(ReplyJson)


class FixedUuid:
    def __init__(self, chunk_id: str) -> None:
        short = chunk_id.removeprefix("chatcmpl-")
        self.value = UUID(hex=f"{short}-0000-4000-8000-000000000000")

    def __call__(self) -> UUID:
        return self.value


class FixedTime:
    def __init__(self, created: int) -> None:
        self.created = created

    def __call__(self) -> float:
        return float(self.created)


class FixedStream(httpx.AsyncByteStream):
    def __init__(self, body: str, split: int | None) -> None:
        self.body = body
        self.split = split

    async def __aiter__(self) -> AsyncIterator[bytes]:
        encoded = self.body.encode()
        if self.split is None:
            yield encoded
            return
        yield encoded[: self.split]
        yield encoded[self.split :]


class FakeClient:
    def __init__(self, status: int, body: str, split: int | None = None) -> None:
        self.status = status
        self.body = body
        self.split = split
        self.url = ""
        self.headers: JsonValue = None
        self.payload: JsonValue = None
        self.streaming = False

    async def post(
        self,
        url: str,
        *,
        headers: JsonValue = None,
        data: str = "",
        stream: bool = False,
    ) -> httpx.Response:
        self.url = url
        self.headers = headers
        self.payload = JSON.validate_json(data)
        self.streaming = stream
        return httpx.Response(
            self.status,
            stream=FixedStream(self.body, self.split),
            request=httpx.Request("POST", url),
        )


class ClientFactory:
    def __init__(self, client: FakeClient) -> None:
        self.client = client
        self.ssl_verify: JsonValue = None

    def __call__(
        self, *, llm_provider: JsonValue, params: JsonObject | None = None
    ) -> FakeClient:
        del llm_provider
        if params is not None:
            self.ssl_verify = params.get("ssl_verify")
        return self.client


class Recorder:
    def __init__(self, *, fail: bool) -> None:
        self.fail = fail
        self.calls: list[JsonObject] = []

    def count(
        self,
        *,
        model: str,
        messages: list[dict[str, object]] | None = None,
        text: str | None = None,
        count_response_tokens: bool = False,
    ) -> int:
        self.calls.append(
            {
                "model": model,
                "messages": JSON.validate_python(messages),
                "text": text,
                "count_response_tokens": count_response_tokens,
            }
        )
        if self.fail:
            raise ValueError("the fixture counter cannot count")
        if messages is not None:
            return len(messages)
        if text is not None:
            return len(text)
        return 0


def patch_identity(chunk_id: str, created: int) -> None:
    uuid.uuid4 = FixedUuid(chunk_id)
    time.time = FixedTime(created)


def install_client(client: FakeClient) -> ClientFactory:
    factory = ClientFactory(client)
    litellm.llms.custom_httpx.http_handler.get_async_httpx_client = factory  # type: ignore[assignment]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L589-L592
    return factory


def extraction_answer(call: ExtractionCall) -> JsonValue:
    handler = AgentHandler()
    content, annotations = handler._extract_content_from_messages(call["messages"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L158-L175
    return {"content": content, "annotations": JSON.validate_python(annotations)}


def annotation_answer(call: AnnotationCall) -> JsonValue:
    handler = AgentHandler()
    transformed = handler._transform_annotations(call["annotations"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L177-L210
    return JSON.validate_python(transformed)


def usage_of(usage: ExistingUsage) -> Usage:
    return Usage(
        prompt_tokens=usage["prompt_tokens"],
        completion_tokens=usage["completion_tokens"],
        total_tokens=usage["total_tokens"],
    )


def usage_view(usage: JsonObject | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def reply_view(response: ModelResponse) -> JsonValue:
    dumped = REPLY_JSON.validate_json(response.model_dump_json())
    first = dumped["choices"][0]
    message = first["message"]
    hidden = cast(JsonObject, response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L226-L230
    return {
        "model": dumped.get("model"),
        "choices": [
            {
                "index": first.get("index"),
                "finish_reason": first.get("finish_reason"),
                "message": {
                    "role": message.get("role"),
                    "content": message.get("content"),
                    "annotations": message.get("annotations"),
                },
            }
        ],
        "hidden_params": hidden,
        "usage": usage_view(dumped.get("usage")),
    }


def reply_answer(call: ReplyCall) -> JsonValue:
    handler = AgentHandler()
    response = ModelResponse()
    if call["existing_usage"] is not None:
        response.usage = usage_of(call["existing_usage"])
    if call["hidden_params"] is not None:
        response._hidden_params = cast(JsonObject, call["hidden_params"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L226-L230
    recorder = Recorder(fail=call["fail"])
    litellm.utils.token_counter = recorder.count  # type: ignore[assignment]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L231-L232
    built = handler._build_model_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L212-L256
        model=call["model"],
        content=call["content"],
        model_response=response,
        thread_id=call["thread_id"],
        messages=call["messages"],
        annotations=call["annotations"],
    )
    return {
        "reply": reply_view(built),
        "counts": JSON.validate_python(recorder.calls),
    }


def chunk_view(chunk: ModelResponseStream) -> JsonValue:
    dumped = CHUNK_JSON.validate_json(chunk.model_dump_json())
    first = dumped["choices"][0]
    delta = first["delta"]
    hidden = cast(JsonObject, chunk._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L655-L703
    return {
        "id": dumped.get("id"),
        "created": dumped.get("created"),
        "model": dumped.get("model"),
        "object": dumped.get("object"),
        "choices": [
            {
                "index": first.get("index"),
                "finish_reason": first.get("finish_reason"),
                "delta": {
                    "content": delta.get("content"),
                    "role": delta.get("role"),
                    "annotations": delta.get("annotations"),
                },
            }
        ],
        "thread_id": hidden.get("thread_id"),
    }


async def opened_stream(
    handler: AgentHandler, plan: StreamPlan
) -> AsyncIterator[ModelResponseStream]:
    stream = handler.acompletion_stream(
        model=plan["model"],
        messages=plan["messages"],
        api_base=plan["api_base"],
        api_key=plan["api_key"],
        logging_obj=None,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L563
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
        "optional_params": EMPTY,
        "litellm_params": EMPTY,
        "headers": {},
    }


async def collect_chunks(
    handler: AgentHandler, call: StreamCall
) -> list[ModelResponseStream]:
    chunks: list[ModelResponseStream] = []
    async for chunk in opened_stream(handler, stream_plan(call)):
        chunks.append(chunk)
    return chunks


def stream_answer(call: StreamCall) -> JsonValue:
    handler = AgentHandler()
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


def request_answer(call: RequestCall) -> JsonValue:
    handler = AgentHandler()
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


def run(argument: Input) -> JsonValue:
    answer: JsonObject = {}
    if "extractions" in argument:
        answer["extractions"] = [extraction_answer(call) for call in argument["extractions"]]
    if "annotations" in argument:
        answer["annotations"] = [annotation_answer(call) for call in argument["annotations"]]
    if "replies" in argument:
        answer["replies"] = [reply_answer(call) for call in argument["replies"]]
    if "streams" in argument:
        answer["streams"] = [stream_answer(call) for call in argument["streams"]]
    if "requests" in argument:
        answer["requests"] = [request_answer(call) for call in argument["requests"]]
    return answer
