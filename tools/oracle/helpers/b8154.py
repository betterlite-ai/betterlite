from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import TYPE_CHECKING, Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.triton.common_utils import TritonError
from litellm.llms.triton.completion import transformation
from litellm.llms.triton.completion.transformation import (
    TritonConfig,
    TritonGenerateConfig,
    TritonInferConfig,
    TritonResponseIterator,
)
from litellm.llms.triton.embedding.transformation import TritonEmbeddingConfig
from litellm.types.utils import EmbeddingResponse, ModelResponse

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllEmbeddingInputValues

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Object: TypeAlias = dict[str, Json]
COMPLETION = TritonConfig()
GENERATE = TritonGenerateConfig()
INFER = TritonInferConfig()
EMBEDDING = TritonEmbeddingConfig()
LLM_TYPE = cast(
    "Callable[[str], str]",
    COMPLETION._get_triton_llm_type,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
)
COMPLETE_URL = cast("Callable[..., str]", COMPLETION.get_complete_url)
MAP_PARAMS = cast("Callable[..., Object]", COMPLETION.map_openai_params)
SUPPORTED = cast("Callable[..., list[str]]", COMPLETION.get_supported_openai_params)
VALIDATE = cast("Callable[..., Object]", COMPLETION.validate_environment)
GENERATE_REQUEST = cast("Callable[..., Object]", GENERATE.transform_request)
GENERATE_REPLY = cast("Callable[..., ModelResponse]", GENERATE.transform_response)
INFER_REQUEST = cast("Callable[..., Object]", INFER.transform_request)
INFER_REPLY = cast("Callable[..., ModelResponse]", INFER.transform_response)
EMBEDDING_REQUEST = cast("Callable[..., Object]", EMBEDDING.transform_embedding_request)
EMBEDDING_REPLY = cast(
    "Callable[..., EmbeddingResponse]", EMBEDDING.transform_embedding_response
)
EMBEDDING_SUPPORTED = cast("Callable[..., list[str]]", EMBEDDING.get_supported_openai_params)
EMBEDDING_VALIDATE = cast("Callable[..., Object]", EMBEDDING.validate_environment)


class ChoiceMessage(Protocol):
    content: str | None


class Choice(Protocol):
    message: ChoiceMessage


class Completion(Protocol):
    choices: list[Choice]


class EmbeddingUsage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class EmbeddingCompletion(Protocol):
    model: str | None
    data: list[Json]
    usage: EmbeddingUsage


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def status_of(error: Exception) -> int:
    if isinstance(error, TritonError):
        return int(error.status_code)
    return 500


@contextmanager
def offline() -> Generator[None]:
    with (
        patch.object(
            huggingface_template_handler, "_get_tokenizer_config", side_effect=RuntimeError
        ),
        patch.object(
            huggingface_template_handler, "_get_chat_template_file", side_effect=RuntimeError
        ),
    ):
        yield


def response(status: Json, body: Json) -> httpx.Response:
    return httpx.Response(
        cast("int", status),
        content=expect_text(body).encode(),
        request=httpx.Request("POST", "https://host"),
    )


def validated_headers(validate: Callable[..., Object]) -> Json:
    validated = validate(
        headers={}, model="model", messages=[], optional_params={}, litellm_params={}
    )
    return {str(name).lower(): value for name, value in validated.items()}


def llm_type(input_value: Object) -> Json:
    found: list[Json] = []
    for base in [expect_text(base) for base in expect_list(input_value["bases"])]:
        try:
            found.append({"ok": True, "llm_type": LLM_TYPE(base)})
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            found.append({"ok": False, "status": status_of(error)})
    found_urls: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["urls"])]:
        try:
            found_urls.append(
                {
                    "ok": True,
                    "url": COMPLETE_URL(
                        api_base=cast("str | None", entry.get("api_base")),
                        api_key=None,
                        model="model",
                        optional_params={},
                        litellm_params={},
                        stream=cast("bool", entry["stream"]),
                    ),
                }
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            found_urls.append({"ok": False, "status": status_of(error)})
    mapped: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["map"])]:
        options = MAP_PARAMS(
            non_default_params=dict(expect_object(entry["non_default"])),
            optional_params=dict(expect_object(entry.get("optional") or {})),
            model="model",
            drop_params=False,
        )
        mapped.append({"options": cast("Json", options)})
    return cast(
        "Json",
        {
            "bases": found,
            "urls": found_urls,
            "supported": SUPPORTED(model="model"),
            "mapped": mapped,
            "validated": validated_headers(VALIDATE),
        },
    )


def transform_requests(input_value: Object, transform: Callable[..., Object]) -> Json:
    bodies: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["requests"])]:
        try:
            body = transform(
                model=expect_text(entry["model"]),
                messages=cast("list[dict[str, Json]]", expect_list(entry["messages"])),
                optional_params=dict(expect_object(entry.get("options") or {})),
                litellm_params={},
                headers={},
            )
            bodies.append({"ok": True, "body": cast("Json", body)})
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            bodies.append({"ok": False, "status": status_of(error)})
    return {"bodies": bodies}


def generate_request(input_value: Object) -> Json:
    configured = expect_object(input_value.get("env") or {}).get("DEFAULT_MAX_TOKENS_FOR_TRITON")
    default = 2000 if configured is None else int(expect_text(configured))
    with offline(), patch.object(transformation, "DEFAULT_MAX_TOKENS_FOR_TRITON", default):
        return transform_requests(input_value, GENERATE_REQUEST)


def transform_replies(input_value: Object, transform: Callable[..., ModelResponse]) -> Json:
    found: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["replies"])]:
        try:
            result = cast(
                "Completion",
                transform(
                    model="model",
                    raw_response=response(entry["status"], entry["body"]),
                    model_response=ModelResponse(),
                    logging_obj=None,
                    request_data={},
                    messages=[],
                    optional_params={},
                    litellm_params={},
                    encoding=None,
                ),
            )
            found.append({"ok": True, "text": result.choices[0].message.content})
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            found.append({"ok": False, "status": status_of(error)})
    return {"replies": found}


def stream_line(input_value: Object) -> Json:
    iterator = TritonResponseIterator(
        streaming_response=iter([]), sync_stream=True, json_mode=False
    )
    handle = cast(
        "Callable[[str], dict[str, Json]]",
        iterator._handle_string_chunk,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    )
    chunks: list[Json] = []
    for line in [expect_text(item) for item in expect_list(input_value["lines"])]:
        try:
            chunk = handle(line)
            chunks.append(
                {
                    "ok": True,
                    "text": chunk.get("text"),
                    "is_finished": chunk.get("is_finished"),
                    "finish_reason": chunk.get("finish_reason"),
                    "index": chunk.get("index"),
                }
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            chunks.append({"ok": False, "status": status_of(error)})
    return {"chunks": chunks}


def _words(model: str, text: str) -> int:
    del model
    return len(text.split())


@patch("litellm.llms.triton.embedding.transformation.token_counter", new=_words)
def embedding(input_value: Object) -> Json:
    inputs = [
        cast("AllEmbeddingInputValues", entry)
        for entry in expect_list(input_value["inputs"])
    ]
    requests: list[Json] = []
    for entry in inputs:
        body = EMBEDDING_REQUEST(model="model", input=entry, optional_params={}, headers={})
        requests.append({"ok": True, "body": cast("Json", body)})
    found: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["replies"])]:
        request = EMBEDDING_REQUEST(
            model="model",
            input=cast("AllEmbeddingInputValues", entry["input"]),
            optional_params={},
            headers={},
        )
        try:
            result = cast(
                "EmbeddingCompletion",
                EMBEDDING_REPLY(
                    model=expect_text(entry["model"]),
                    raw_response=response(entry["status"], entry["body"]),
                    model_response=EmbeddingResponse(),
                    logging_obj=None,
                    api_key=None,
                    request_data=request,
                    optional_params={},
                    litellm_params={},
                ),
            )
            found.append(
                {
                    "ok": True,
                    "model": result.model,
                    "data": result.data,
                    "usage": {
                        "prompt_tokens": result.usage.prompt_tokens,
                        "completion_tokens": result.usage.completion_tokens,
                        "total_tokens": result.usage.total_tokens,
                    },
                }
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            found.append({"ok": False, "status": status_of(error)})
    split_results: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["split"])]:
        try:
            chunks = EMBEDDING.split_embedding_by_shape(
                cast("list[float]", entry["data"]), cast("list[int]", entry["shape"])
            )
            split_results.append(cast("Json", {"ok": True, "chunks": chunks}))
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            split_results.append({"ok": False, "status": status_of(error)})
    return cast(
        "Json",
        {
            "supported": EMBEDDING_SUPPORTED(model="model"),
            "headers": validated_headers(EMBEDDING_VALIDATE),
            "requests": requests,
            "replies": found,
            "split": split_results,
        },
    )


OPS: dict[str, Callable[[Object], Json]] = {
    "llm_type": llm_type,
    "generate_request": generate_request,
    "infer_request": lambda value: transform_requests(value, INFER_REQUEST),
    "generate_reply": lambda value: transform_replies(value, GENERATE_REPLY),
    "infer_reply": lambda value: transform_replies(value, INFER_REPLY),
    "stream_line": stream_line,
    "embedding": embedding,
}


def run(input_value: Object) -> Json:
    operation = expect_text(input_value["op"])
    return OPS[operation](input_value)
