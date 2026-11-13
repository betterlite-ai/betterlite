import json
from collections.abc import Callable, Iterator
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.nlp_cloud.chat import handler, transformation
from litellm.llms.nlp_cloud.chat.handler import (
    clean_and_iterate_chunks,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.llms.nlp_cloud.chat.transformation import NLPCloudConfig
from litellm.llms.nlp_cloud.common_utils import NLPCloudError
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
CONFIG_DEFAULTS: tuple[str, ...] = (
    "max_length",
    "length_no_input",
    "end_sequence",
    "remove_end_sequence",
    "remove_input",
    "bad_words",
    "temperature",
    "top_p",
    "top_k",
    "repetition_penalty",
    "num_beams",
    "num_return_sequences",
)
DEFAULT_REPLY = '{"generated_text":"ok","nb_input_tokens":0,"nb_generated_tokens":0}'
COMPLETION = cast("Callable[..., object]", handler.completion)


class ApiError(Protocol):
    status_code: int
    message: str
    headers: dict[str, str] | None


class ChoiceMessage(Protocol):
    content: str | None


class Choice(Protocol):
    message: ChoiceMessage
    finish_reason: str


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Completion(Protocol):
    choices: list[Choice]
    created: int
    model: str
    usage: Usage


class ChatConfig(Protocol):
    def get_config(self) -> dict[str, Json]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def validate_environment(  # noqa: PLR0913 - mirrors LiteLLM's parameter list
        self,
        headers: dict[str, Json],
        model: str,
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        *,
        api_key: str | None,
    ) -> dict[str, Json]: ...

    def get_error_class(
        self,
        error_message: str,
        status_code: int,
        headers: dict[str, str] | None,
    ) -> ApiError: ...

    def transform_response(  # noqa: PLR0913 - mirrors LiteLLM's parameter list
        self,
        *,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: object,
        request_data: dict[str, Json],
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        encoding: None,
    ) -> Completion: ...


class Logging:
    def pre_call(self, **params: Json) -> None:
        del params

    def post_call(self, **params: Json) -> None:
        del params


class RecordingClient:
    def __init__(self, response: object) -> None:
        self.response = response
        self.calls: list[Object] = []

    def post(
        self,
        url: str,
        headers: dict[str, str] | None = None,
        data: str | None = None,
        *,
        stream: bool = False,
    ) -> object:
        self.calls.append(
            {
                "url": url,
                "headers": cast("Json", {} if headers is None else dict(headers)),
                "body": {} if data is None else cast("Json", json.loads(data)),
                "stream": stream,
            }
        )
        return self.response


class ChunkedResponse:
    def __init__(self, chunks: list[str]) -> None:
        self.chunks = chunks

    def iter_content(self, chunk_size: int) -> Iterator[bytes]:
        del chunk_size
        for chunk in self.chunks:
            yield chunk.encode()


def verbose(*args: Json, **kwargs: Json) -> None:
    del args, kwargs


def config() -> ChatConfig:
    return cast("ChatConfig", NLPCloudConfig())


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def obj_or_empty(value: Json | None) -> Object:
    return {} if value is None else obj(value)


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def expect_int(fields: Object, name: str) -> int:
    value = fields.get(name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be an integer")
    return value


def message_list(fields: Object) -> list[Json]:
    value = fields.get("messages")
    if not isinstance(value, list):
        raise TypeError("messages must be a list")
    return value


def text_list(value: Json) -> list[str]:
    if not isinstance(value, list):
        raise TypeError("expected a list of strings")
    texts: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise TypeError("expected a list of strings")
        texts.append(item)
    return texts


def optional_headers(fields: Object, name: str) -> dict[str, str] | None:
    value = fields.get(name)
    if value is None:
        return None
    headers: dict[str, str] = {}
    for key, item in obj(value).items():
        if not isinstance(item, str):
            raise TypeError(f"{name} values must be strings")
        headers[key] = item
    return headers


def apply_defaults(defaults: Object) -> None:
    for name in CONFIG_DEFAULTS:
        setattr(NLPCloudConfig, name, defaults.get(name))


def status_of(error: Exception) -> int:
    if isinstance(error, NLPCloudError):
        return int(error.status_code)
    return 500


def project(result: Completion) -> Json:
    choice = result.choices[0]
    return {
        "text": choice.message.content,
        "finish_reason": choice.finish_reason,
        "created": result.created,
        "model": result.model,
        "usage": {
            "prompt_tokens": result.usage.prompt_tokens,
            "completion_tokens": result.usage.completion_tokens,
            "total_tokens": result.usage.total_tokens,
        },
    }


def stream_chunks(result: object) -> list[str]:
    return list(cast("Iterator[str]", result))


def make_response(step: Object, chunks: Json | None) -> object:
    if chunks is not None:
        return ChunkedResponse(text_list(chunks))
    reply = step.get("reply")
    if reply is None:
        return httpx.Response(
            200,
            content=DEFAULT_REPLY.encode(),
            request=httpx.Request("POST", "https://host"),
        )
    fields = obj(reply)
    return httpx.Response(
        expect_int(fields, "status"),
        content=text(fields, "body").encode(),
        request=httpx.Request("POST", "https://host"),
    )


def supported_params(_step: Object) -> Json:
    return cast("Json", config().get_supported_openai_params(""))


def map_params(step: Object) -> Json:
    params = obj(step.get("params"))
    options = obj_or_empty(step.get("optional_params"))
    return cast(
        "Json", config().map_openai_params(params, options, "", drop_params=False)
    )


def headers(step: Object) -> Json:
    return cast(
        "Json",
        config().validate_environment(
            obj_or_empty(step.get("headers")),
            "",
            [],
            {},
            {},
            api_key=optional_text(step, "api_key"),
        ),
    )


def error_class(step: Object) -> Json:
    error = config().get_error_class(
        text(step, "message"),
        expect_int(step, "status"),
        optional_headers(step, "headers"),
    )
    value = error.headers
    rendered: Json = None
    if isinstance(value, httpx.Headers):
        rendered = cast("Json", dict(value))
    elif value is not None:
        rendered = cast("Json", value)
    result: Object = {
        "status_code": error.status_code,
        "message": error.message,
        "headers": rendered,
    }
    return result


def defaults(step: Object) -> Json:
    apply_defaults(obj_or_empty(step.get("defaults")))
    return cast("Json", config().get_config())


def completion(step: Object) -> Json:
    apply_defaults(obj_or_empty(step.get("defaults")))
    options = obj_or_empty(step.get("optional_params"))
    chunks = step.get("chunks")
    client = RecordingClient(make_response(step, chunks))
    clock = float(expect_int(step, "created"))
    with (
        patch.object(handler, "_get_httpx_client", return_value=client),  # pyright: ignore[reportPrivateUsage]
        patch.object(transformation.time, "time", return_value=clock),  # pyright: ignore[reportPrivateUsage]
    ):
        result = COMPLETION(
            model=text(step, "model"),
            messages=message_list(step),
            api_base=text(step, "api_base"),
            model_response=ModelResponse(),
            print_verbose=verbose,
            encoding=None,
            api_key=optional_text(step, "api_key"),
            logging_obj=Logging(),
            optional_params=options,
            litellm_params={},
            headers=obj_or_empty(step.get("headers")),
        )
    call = client.calls[0]
    output: Object = {
        "ok": True,
        "url": call["url"],
        "headers": call["headers"],
        "body": call["body"],
        "reply": None if chunks is not None else project(cast("Completion", result)),
        "chunks": None if chunks is None else cast("Json", stream_chunks(result)),
    }
    return output


def reply(step: Object) -> Json:
    response = httpx.Response(
        expect_int(step, "status"),
        content=text(step, "body").encode(),
        request=httpx.Request("POST", "https://host"),
    )
    clock = float(expect_int(step, "created"))
    try:
        with patch.object(transformation.time, "time", return_value=clock):  # pyright: ignore[reportPrivateUsage]
            result = config().transform_response(
                model=text(step, "model"),
                raw_response=response,
                model_response=ModelResponse(),
                logging_obj=Logging(),
                request_data={},
                messages=[],
                optional_params={},
                litellm_params={},
                encoding=None,
            )
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return {"ok": False, "status": status_of(error)}
    return {"ok": True, "reply": project(result)}


def stream_lines(step: Object) -> Json:
    value = step.get("lines")
    if value is None:
        raise TypeError("lines must be a list")
    cleaner = cast(
        "Callable[[ChunkedResponse], Iterator[str]]", clean_and_iterate_chunks
    )
    return cast("Json", list(cleaner(ChunkedResponse(text_list(value)))))


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "supported_params": supported_params,
    "map_params": map_params,
    "headers": headers,
    "error_class": error_class,
    "defaults": defaults,
    "completion": completion,
    "reply": reply,
    "stream_lines": stream_lines,
}


def execute(step: Object) -> Json:
    kind = text(step, "kind")
    found = HANDLERS.get(kind)
    if found is None:
        raise ValueError(f"unknown step kind {kind}")
    return found(step)


def run(value: dict[str, Json]) -> Json:
    steps = value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    return [execute(obj(step)) for step in steps]
