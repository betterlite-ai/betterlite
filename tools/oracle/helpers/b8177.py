import json
import uuid
from collections.abc import Callable, Iterator, Mapping
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.gigachat.chat import streaming, transformation
from litellm.llms.gigachat.chat.streaming import GigaChatModelResponseIterator
from litellm.llms.gigachat.chat.transformation import GigaChatConfig
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

FIXED_UUID = uuid.UUID("01234567-89ab-4def-8123-456789abcdef")


class FixedUuidModule:
    def uuid4(self) -> uuid.UUID:
        return FIXED_UUID


class PromptDetails(Protocol):
    cached_tokens: int | None


class UsageRecord(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: PromptDetails | None


class ToolFunction(Protocol):
    name: str
    arguments: str


class ToolCallRecord(Protocol):
    id: str
    type: str
    function: ToolFunction


class ChoiceMessage(Protocol):
    role: str
    content: str | None
    tool_calls: list[ToolCallRecord] | None


class Choice(Protocol):
    index: int
    message: ChoiceMessage
    finish_reason: str | None


class Reply(Protocol):
    id: str
    created: int
    model: str
    choices: list[Choice]
    usage: UsageRecord


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: Mapping[str, object],
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
        optional_params: Mapping[str, object],
        litellm_params: Mapping[str, object],
        *,
        api_key: str | None,
    ) -> dict[str, Json]: ...

    def _transform_messages(self, messages: list[Json]) -> list[Json]: ...

    def get_complete_url(  # noqa: PLR0913 - mirrors LiteLLM's parameter list
        self,
        *,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Mapping[str, object],
        litellm_params: Mapping[str, object],
        stream: bool | None,
    ) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[Json],
        optional_params: Mapping[str, object],
        litellm_params: Mapping[str, object],
        headers: Mapping[str, Json],
    ) -> dict[str, Json]: ...

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
    ) -> Reply: ...


class TokenSpy:
    def __init__(self, token: str) -> None:
        self.token = token
        self.seen: list[str | None] = []

    def __call__(
        self,
        credentials: str | None = None,
        scope: str | None = None,
        auth_url: str | None = None,
        litellm_params: Mapping[str, object] | None = None,
    ) -> str:
        del scope, auth_url, litellm_params
        self.seen.append(credentials)
        return self.token


class UploadSpy:
    def __init__(self, ids: dict[str, str]) -> None:
        self.ids = ids

    def __call__(
        self,
        image_url: str,
        credentials: str | None = None,
        api_base: str | None = None,
        litellm_params: Mapping[str, object] | None = None,
    ) -> str | None:
        del credentials, api_base, litellm_params
        return self.ids.get(image_url)


class Logging:
    def pre_call(self, **params: Json) -> None:
        del params

    def post_call(self, **params: Json) -> None:
        del params


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


def text_list(value: Json) -> list[str]:
    if not isinstance(value, list):
        raise TypeError("expected a list of strings")
    texts: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise TypeError("expected a list of strings")
        texts.append(item)
    return texts


def message_list(fields: Object) -> list[Json]:
    value = fields.get("messages")
    if not isinstance(value, list):
        raise TypeError("messages must be a list")
    return value


def optional_headers(fields: Object, name: str) -> dict[str, Json] | None:
    value = fields.get(name)
    if value is None:
        return None
    headers: dict[str, Json] = {}
    for key, item in obj(value).items():
        if not isinstance(item, str):
            raise TypeError(f"{name} values must be strings")
        headers[key] = item
    return headers


def upload_spy(step: Object) -> UploadSpy:
    ids: dict[str, str] = {}
    for url, file_id in obj_or_empty(step.get("uploads")).items():
        if not isinstance(file_id, str):
            raise TypeError("upload ids must be strings")
        ids[url] = file_id
    return UploadSpy(ids)


def config() -> ChatConfig:
    return cast("ChatConfig", GigaChatConfig())


def status_of(error: Exception) -> int | None:
    carried = getattr(error, "status_code", None)
    if isinstance(carried, int) and not isinstance(carried, bool):
        return carried
    return None


def failure(error: Exception) -> Json:
    status = status_of(error)
    if status is None:
        return {"ok": False}
    return {"ok": False, "status_code": status}


def parse_arguments(arguments: Json) -> Json:
    if not isinstance(arguments, str):
        return arguments
    try:
        return cast("Json", json.loads(arguments))
    except json.JSONDecodeError:
        return arguments


def project_usage(usage: UsageRecord) -> Json:
    details = usage.prompt_tokens_details
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "cached_tokens": None if details is None else details.cached_tokens,
    }


def project_reply(response: Reply) -> Object:
    choices: list[Json] = []
    for choice in response.choices:
        message = choice.message
        tool_calls: Json = None
        if message.tool_calls is not None:
            tool_calls = [
                {
                    "id": cast("Json", call.id),
                    "type": cast("Json", call.type),
                    "function": {
                        "name": cast("Json", call.function.name),
                        "arguments": parse_arguments(call.function.arguments),
                    },
                }
                for call in message.tool_calls
            ]
        choices.append(
            {
                "index": cast("Json", choice.index),
                "role": cast("Json", message.role),
                "content": cast("Json", message.content),
                "tool_calls": tool_calls,
                "finish_reason": cast("Json", choice.finish_reason),
            }
        )
    return {
        "id": cast("Json", response.id),
        "created": cast("Json", response.created),
        "model": cast("Json", response.model),
        "choices": choices,
        "usage": project_usage(response.usage),
    }


def supported_params(_step: Object) -> Json:
    return {"ok": True, "params": cast("Json", config().get_supported_openai_params(""))}


def map_params(step: Object) -> Json:
    params = obj_or_empty(step.get("params"))
    options = obj_or_empty(step.get("optional_params"))
    try:
        mapped = config().map_openai_params(params, dict(options), "", drop_params=False)
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return failure(error)
    return {"ok": True, "params": cast("Json", mapped)}


def headers(step: Object) -> Json:
    spy = TokenSpy(text(step, "token"))
    caller = optional_headers(step, "headers") or {}
    with patch.object(transformation, "get_access_token", spy):
        try:
            result = config().validate_environment(
                caller, "", [], {}, {}, api_key=optional_text(step, "api_key")
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return {
        "ok": True,
        "credentials": spy.seen[0] if spy.seen else None,
        "headers": cast("Json", result),
    }


def messages(step: Object) -> Json:
    spy = upload_spy(step)
    with patch.object(transformation, "upload_file_sync", spy):
        try:
            turned = config()._transform_messages(message_list(step))  # noqa: SLF001 - the case compares this method  # pyright: ignore[reportPrivateUsage]
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return {"ok": True, "messages": cast("Json", list(turned))}


def request(step: Object) -> Json:
    spy = TokenSpy(text(step, "token"))
    uploads = upload_spy(step)
    model = text(step, "model")
    params = obj_or_empty(step.get("params"))
    caller = optional_headers(step, "headers") or {}
    with (
        patch.object(transformation, "get_access_token", spy),
        patch.object(transformation, "upload_file_sync", uploads),
    ):
        try:
            sent = config().validate_environment(
                caller, model, message_list(step), params, {}, api_key=None
            )
            url = config().get_complete_url(
                api_base=optional_text(step, "api_base"),
                api_key=None,
                model=model,
                optional_params=params,
                litellm_params={},
                stream=None,
            )
            body = config().transform_request(
                model, message_list(step), params, {}, sent
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return {
        "ok": True,
        "url": url,
        "headers": cast("Json", sent),
        "body": cast("Json", body),
    }


def reply(step: Object) -> Json:
    response = httpx.Response(
        expect_int(step, "status"),
        content=text(step, "body").encode(),
        request=httpx.Request("POST", "https://host"),
    )
    clock = float(expect_int(step, "created"))
    options: Object = (
        {"_structured_output": True} if step.get("structured_output") is True else {}
    )
    with (
        patch.object(transformation.time, "time", return_value=clock),  # pyright: ignore[reportPrivateImportUsage]
        patch.object(transformation, "uuid", FixedUuidModule()),  # pyright: ignore[reportPrivateUsage]
    ):
        try:
            result = config().transform_response(
                model=text(step, "model"),
                raw_response=response,
                model_response=ModelResponse(),
                logging_obj=Logging(),
                request_data={},
                messages=[],
                optional_params=options,
                litellm_params={},
                encoding=None,
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    projected = project_reply(result)
    return {"ok": True, **projected}


def project_tool_use(value: Json) -> Json:
    if value is None:
        return None
    tool = obj(value)
    function = obj(tool.get("function"))
    return {
        "id": tool.get("id"),
        "type": tool.get("type"),
        "function": {
            "name": function.get("name"),
            "arguments": parse_arguments(function.get("arguments")),
        },
        "index": tool.get("index"),
    }


def project_block(value: Json) -> Json:
    if value is None:
        return None
    usage = obj(value)
    details = usage.get("prompt_tokens_details")
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
        "cached_tokens": None if details is None else obj(details).get("cached_tokens"),
    }


def project_chunk(chunk: Mapping[str, Json]) -> Json:
    return {
        "text": chunk.get("text"),
        "tool_use": project_tool_use(chunk.get("tool_use")),
        "is_finished": chunk.get("is_finished"),
        "finish_reason": chunk.get("finish_reason"),
        "usage": project_block(chunk.get("usage")),
        "index": chunk.get("index"),
    }


def stream(step: Object) -> Json:
    events = text_list(step.get("events"))
    with patch.object(streaming, "uuid", FixedUuidModule()):  # pyright: ignore[reportPrivateUsage]
        iterator = cast(
            "Iterator[Mapping[str, Json]]",
            GigaChatModelResponseIterator(iter(events), sync_stream=True),
        )
        try:
            chunks = [project_chunk(chunk) for chunk in iterator]
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return {"ok": True, "chunks": chunks}


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "supported_params": supported_params,
    "map_params": map_params,
    "headers": headers,
    "messages": messages,
    "request": request,
    "reply": reply,
    "stream": stream,
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
