from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gigachat.embedding.transformation import GigaChatEmbeddingConfig
from litellm.llms.gigachat.passthrough.transformation import GigaChatPassthroughConfig
from litellm.types.llms.openai import AllEmbeddingInputValues, AllMessageValues
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

from xai_files import JSON, guarded

Object: TypeAlias = dict[str, JsonValue]
EMPTY: Final[Object] = {}
URL_MODEL: Final = "Embeddings"
litellm.suppress_debug_info = True


class NoMessages(list[AllMessageValues]): ...


class HeadersConfig(Protocol):
    def validate_environment(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gigachat/passthrough/transformation.py#L50-L69
        self, headers: dict[str, str], model: str, messages: NoMessages,
        optional_params: dict[str, JsonValue], litellm_params: dict[str, JsonValue],
        api_key: str | None, /,
    ) -> dict[str, str]: ...


class EmbeddingConfig(HeadersConfig, Protocol):
    def get_complete_url(
        self, api_base: str | None, api_key: str | None, model: str,
        optional_params: dict[str, JsonValue], litellm_params: dict[str, JsonValue], /,
    ) -> str: ...

    def transform_embedding_request(
        self, model: str, embedding_input: AllEmbeddingInputValues,
        optional_params: dict[str, JsonValue], headers: dict[str, JsonValue], /,
    ) -> dict[str, JsonValue]: ...

    def transform_embedding_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gigachat/embedding/transformation.py#L121-L131
        self, model: str, raw_response: httpx.Response, model_response: EmbeddingResponse,
        logging_obj: Logging, api_key: str | None, request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue], litellm_params: dict[str, JsonValue], /,
    ) -> EmbeddingResponse: ...


EMBEDDING: Final[EmbeddingConfig] = cast(EmbeddingConfig, GigaChatEmbeddingConfig())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gigachat/embedding/transformation.py#L30-L197
CONFIG: Final = GigaChatPassthroughConfig()
VALIDATE: Final[HeadersConfig] = cast(HeadersConfig, CONFIG)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gigachat/passthrough/transformation.py#L50-L69
LOG: Final = Logging(
    model="", messages=None, stream=False, call_type="", start_time=datetime.now(UTC),
    litellm_call_id="", function_id="",
)


class EmbeddingRequestStep(TypedDict):
    kind: Literal["embedding_request"]
    model: str
    input: AllEmbeddingInputValues
    api_base: str | None
    env_base: str | None
    token: str
    headers: dict[str, str]


class EmbeddingReplyStep(TypedDict):
    kind: Literal["embedding_reply"]
    body: str


class UrlStep(TypedDict):
    kind: Literal["passthrough_url"]
    api_base: str | None
    env_base: str | None
    endpoint: str


class HeadersStep(TypedDict):
    kind: Literal["passthrough_headers"]
    headers: dict[str, str]
    token: str | None


class KeyStep(TypedDict):
    kind: Literal["passthrough_key"]
    api_key: str | None
    env_key: str | None


class StreamingStep(TypedDict):
    kind: Literal["passthrough_streaming"]
    request_data: Object


class LoggingStep(TypedDict):
    kind: Literal["passthrough_logging"]
    endpoint: str
    model: str
    status: int
    body: str


class CollectedStep(TypedDict):
    kind: Literal["passthrough_collected"]
    chunks: list[str]
    model: str
    endpoint: str


Step: TypeAlias = (
    EmbeddingRequestStep | EmbeddingReplyStep | UrlStep | HeadersStep
    | KeyStep | StreamingStep | LoggingStep | CollectedStep
)


class Input(TypedDict):
    steps: list[JsonValue]


class UsageDump(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class MessageDump(TypedDict):
    role: str
    content: str | None


class ChoiceDump(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageDump


class ChatDump(TypedDict):
    id: str
    created: int
    model: str
    choices: list[ChoiceDump]
    usage: UsageDump | None


class EmbedDump(TypedDict):
    object: str
    model: str | None
    data: list[JsonValue]
    usage: UsageDump | None


class MergedChoice(TypedDict):
    finish_reason: str | None
    message: MessageDump


class MergedDump(TypedDict):
    choices: list[MergedChoice]
    usage: UsageDump | None


CHAT: Final[TypeAdapter[ChatDump]] = TypeAdapter(ChatDump)
EMBED: Final[TypeAdapter[EmbedDump]] = TypeAdapter(EmbedDump)
MERGED: Final[TypeAdapter[MergedDump]] = TypeAdapter(MergedDump)
STEPS: Final[TypeAdapter[list[Step]]] = TypeAdapter(list[Step])


def usage_value(usage: UsageDump | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage["prompt_tokens"], "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def embed_usage_value(usage: UsageDump | None) -> JsonValue:
    if usage is None:
        return None
    return {"prompt_tokens": usage["prompt_tokens"], "total_tokens": usage["total_tokens"]}


def choice_value(choice: ChoiceDump) -> JsonValue:
    return {
        "index": choice["index"], "finish_reason": choice["finish_reason"],
        "message": {"role": choice["message"]["role"], "content": choice["message"]["content"]},
    }


def chat_value(dumped: ChatDump) -> JsonValue:
    return {
        "id": dumped["id"], "created": dumped["created"], "model": dumped["model"],
        "choices": [choice_value(choice) for choice in dumped["choices"]],
        "usage": usage_value(dumped["usage"]),
    }


def embed_value(dumped: EmbedDump) -> JsonValue:
    return {
        "object": dumped["object"], "model": dumped["model"], "data": dumped["data"],
        "usage": embed_usage_value(dumped["usage"]),
    }


def embedding_request(step: EmbeddingRequestStep) -> JsonValue:
    headers: dict[str, str] = EMBEDDING.validate_environment(
        step["headers"], step["model"], NoMessages(), EMPTY, EMPTY, None
    )
    url = EMBEDDING.get_complete_url(step["api_base"], None, step["model"], EMPTY, EMPTY)
    body: dict[str, JsonValue] = EMBEDDING.transform_embedding_request(
        step["model"], step["input"], EMPTY, EMPTY
    )
    return {
        "ok": True, "url": url, "headers": JSON.validate_python(headers),
        "body": JSON.validate_python(body),
    }


def embedding_reply(step: EmbeddingReplyStep) -> JsonValue:
    response = httpx.Response(200, content=step["body"].encode())
    reply = EMBEDDING.transform_embedding_response(
        "m", response, EmbeddingResponse(), LOG, None, EMPTY, EMPTY, EMPTY
    )
    return {"ok": True, "value": embed_value(EMBED.validate_json(reply.model_dump_json()))}


def passthrough_url(step: UrlStep) -> JsonValue:
    url, base = CONFIG.get_complete_url(
        step["api_base"], None, URL_MODEL, step["endpoint"], None, EMPTY
    )
    return {"ok": True, "url": str(url), "base": base}


def passthrough_headers(step: HeadersStep) -> JsonValue:
    headers: dict[str, str] = VALIDATE.validate_environment(
        step["headers"], "m", NoMessages(), EMPTY, EMPTY, step["token"]
    )
    return {"ok": True, "headers": JSON.validate_python(headers)}


def passthrough_logging(step: LoggingStep) -> JsonValue:
    response = httpx.Response(step["status"], content=step["body"].encode())
    logged = CONFIG.logging_non_streaming_response(
        step["model"], "gigachat", response, EMPTY, LOG, step["endpoint"]
    )
    if logged is None:
        return {"ok": True, "value": None}
    dumped = logged.model_dump_json()
    if "completions" in step["endpoint"]:
        return {"ok": True, "value": chat_value(CHAT.validate_json(dumped))}
    return {"ok": True, "value": embed_value(EMBED.validate_json(dumped))}


def passthrough_collected(step: CollectedStep) -> JsonValue:
    logged = CONFIG.handle_logging_collected_chunks(
        step["chunks"], LOG, step["model"], "gigachat", step["endpoint"]
    )
    if logged is None:
        return {"ok": True, "value": None}
    dumped = MERGED.validate_json(logged.model_dump_json())
    choices = dumped["choices"]
    if not choices:
        return {"ok": True, "value": None}
    choice = choices[0]
    value = {
        "text": choice["message"]["content"], "finish_reason": choice["finish_reason"],
        "usage": usage_value(dumped["usage"]),
    }
    return {"ok": True, "value": value}


def step_value(step: Step) -> JsonValue:
    result: JsonValue
    if step["kind"] == "embedding_request":
        result = guarded(embedding_request, step)
    elif step["kind"] == "embedding_reply":
        result = guarded(embedding_reply, step)
    elif step["kind"] == "passthrough_url":
        result = guarded(passthrough_url, step)
    elif step["kind"] == "passthrough_headers":
        result = guarded(passthrough_headers, step)
    elif step["kind"] == "passthrough_key":
        result = {"ok": True, "key": GigaChatPassthroughConfig.get_api_key(step["api_key"])}
    elif step["kind"] == "passthrough_streaming":
        result = {"ok": True, "streaming": CONFIG.is_streaming_request("", step["request_data"])}
    elif step["kind"] == "passthrough_logging":
        result = guarded(passthrough_logging, step)
    else:
        result = guarded(passthrough_collected, step)
    return result


def run(input_value: Input) -> JsonValue:
    steps = STEPS.validate_python(input_value["steps"])
    return [step_value(step) for step in steps]
