import json
from collections.abc import Coroutine, Mapping
from functools import partial
from typing import Final, Literal, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import timeout_value
from betterlite._completion.errors import UnsupportedCallArgument, raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeError
from betterlite._native_assistant_calls import (
    NativeAssistantError,
    NativeAssistantSession,
)
from betterlite.types.llms.openai.core.threads import Thread
from betterlite.types.utils.base import Dumpable, ResponseObject

SETTINGS: Final = ("api_key", "api_base", "api_version", "organization", "openai_key", "azure_key")
PARAMS: Final = ("api_key", "api_base", "api_version", "organization", "max_retries", "timeout",
                 "request_timeout", "azure_ad_token", "tenant_id", "client_id", "client_secret",
                 "azure_username", "azure_password", "azure_scope")
DROPPED: Final = ("client", "shared_session", "logger_fn")
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Assistant(ResponseObject):
    id: str
    created_at: int
    description: str | None = None
    instructions: str | None = None
    metadata: dict[str, str] | None = None
    model: str
    name: str | None = None
    object: Literal["assistant"]
    tools: list[JsonValue]
    response_format: JsonValue = None
    temperature: float | None = None
    tool_resources: JsonValue = None
    top_p: float | None = None


class AssistantDeleted(ResponseObject):
    id: str
    deleted: bool
    object: Literal["assistant.deleted"]


class AssistantPage(ResponseObject):
    data: list[Assistant]
    has_more: bool


PageAnswer: TypeAlias = AssistantPage | Coroutine[None, None, AssistantPage]
DeletedAnswer: TypeAlias = AssistantDeleted | Coroutine[None, None, AssistantDeleted]
RecordT = TypeVar("RecordT", bound=ResponseObject)
READ_METHODS: Final = {
    "list": NativeAssistantSession.get_assistants,
    "delete": NativeAssistantSession.delete_assistant,
}


def session() -> NativeAssistantSession:
    settings = json.dumps(read_settings(SETTINGS))
    return NativeAssistantSession(settings)


def flag_argument(name: str, arguments: Mapping[str, CallValue], error: type[Exception]) -> bool:
    value = arguments.get(name)
    if value is None:
        return False
    match value:
        case bool() as flag:
            return flag
        case object():
            raise error(f"Invalid value passed in for {name}. Only bool or None allowed")


def param_value(name: str, value: CallValue) -> JsonValue:
    if name == "timeout":
        return timeout_value(value)
    if name == "api_version":
        match value:
            case None | str():
                return value
            case object():
                raise UnsupportedCallArgument("api_version")
    return JSON.validate_python(value)


def params_of(arguments: Mapping[str, CallValue]) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    for name in PARAMS:
        if name in arguments:
            params[name] = param_value(name, arguments[name])
    return params


def named_arguments(
    arguments: Mapping[str, CallValue], named: Mapping[str, CallValue]) -> dict[str, CallValue]:
    given = {name: value for name, value in named.items() if value is not None}
    return {**arguments, **given}


def thread_message(value: CallValue | Dumpable) -> JsonValue:
    match value:
        case Dumpable() as model:
            return JSON.validate_python(model.model_dump(exclude_none=True, mode="json"))
        case object():
            return JSON.validate_python(value)


def read_delete_payload(
    operation: str, provider: str, entry: str, arguments: Mapping[str, CallValue]) -> CallPayload:
    names = ("assistant_id", "aget_assistants", "async_delete_assistants", "after", "before",
             "limit", "order")
    values: dict[str, CallValue] = {"operation": operation, "provider": provider, "entry": entry}
    values.update({name: arguments.get(name) for name in names})
    values["params"] = params_of(arguments)
    values.update({name: arguments.get(name) for name in DROPPED})
    if "extra_body" in arguments:
        values["extra_body"] = arguments["extra_body"]
    return to_call_payload(values)


def read_delete_answer(record: type[RecordT], operation: Literal["list", "delete"],
                       provider: str, arguments: Mapping[str, CallValue]) -> RecordT:
    method = READ_METHODS[operation]
    payload = read_delete_payload(operation, provider, "sync", arguments)
    return record.model_validate(assistant_answer(partial(method, session()), payload))


def create_payload(provider: str, arguments: Mapping[str, CallValue]) -> CallPayload:
    values: dict[str, CallValue] = {**arguments, "provider": provider}
    for name in ("timeout", "api_version"):
        if name in arguments:
            values[name] = param_value(name, arguments[name])
    return to_call_payload(values)


def thread_payload(
    provider: str, thread_id: str | None, arguments: Mapping[str, CallValue]) -> CallPayload:
    values: dict[str, CallValue] = {"custom_llm_provider": provider, "thread_id": thread_id}
    messages = arguments.get("messages")
    if messages is not None:
        match messages:
            case list() as items:
                values["messages"] = [thread_message(item) for item in items]
            case object():
                raise UnsupportedCallArgument("messages")
    fields = {name: arguments[name] for name in ("metadata", "tool_resources") if name in arguments}
    values.update(fields)
    values.update({name: arguments.get(name) for name in DROPPED})
    kwargs = params_of(arguments)
    if "extra_body" in arguments:
        kwargs["extra_body"] = JSON.validate_python(arguments["extra_body"])
    values["kwargs"] = kwargs
    return to_call_payload(values)


def assistant_answer(method: NativeCall, payload: CallPayload) -> JsonValue:
    try:
        answer = method(payload.text, payload.blobs)
    except NativeAssistantError as failure:
        raise_native_error(NativeError(str(failure)))
    return JSON.validate_json(answer)


async def assistant_aanswer(method: AsyncNativeCall, payload: CallPayload) -> JsonValue:
    try:
        answer = await method(payload.text, payload.blobs)
    except NativeAssistantError as failure:
        raise_native_error(NativeError(str(failure)))
    return JSON.validate_json(answer)


def thread_of(value: JsonValue) -> Thread | None:
    if value is None:
        return None
    match value:
        case dict() as record:
            return Thread.model_validate(record)
        case object():
            raise TypeError("the thread reply is not an object")


def get_assistants(
    custom_llm_provider: str, client: CallValue = None, api_key: CallValue = None,
    api_base: CallValue = None, api_version: CallValue = None, **kwargs: CallValue) -> PageAnswer:
    args = {"client": client, "api_key": api_key, "api_base": api_base, "api_version": api_version}
    arguments = named_arguments(kwargs, args)
    if flag_argument("aget_assistants", kwargs, Exception):
        return aget_assistants(custom_llm_provider, **arguments)
    return read_delete_answer(AssistantPage, "list", custom_llm_provider, arguments)


async def aget_assistants(
    custom_llm_provider: str, client: CallValue = None, **kwargs: CallValue,
) -> AssistantPage:
    arguments = named_arguments(kwargs, {"client": client, "aget_assistants": True})
    payload = read_delete_payload("list", custom_llm_provider, "async", arguments)
    answer = await assistant_aanswer(session().aget_assistants, payload)
    return AssistantPage.model_validate(answer)


def create_assistants(
    custom_llm_provider: str, model: str,
    name: CallValue = None, description: CallValue = None, instructions: CallValue = None,
    tools: CallValue = None, tool_resources: CallValue = None, metadata: CallValue = None,
    temperature: CallValue = None, top_p: CallValue = None, response_format: CallValue = None,
    client: CallValue = None, api_key: CallValue = None, api_base: CallValue = None,
    api_version: CallValue = None, **kwargs: CallValue,
) -> Assistant | Coroutine[None, None, Assistant]:
    flag = flag_argument("async_create_assistants", kwargs, ValueError)
    named: dict[str, CallValue] = {
        "model": model, "name": name, "description": description,
        "instructions": instructions, "tools": tools, "tool_resources": tool_resources,
        "metadata": metadata, "temperature": temperature, "top_p": top_p,
        "response_format": response_format, "client": client, "api_key": api_key,
        "api_base": api_base, "api_version": api_version,
    }
    arguments = named_arguments(kwargs, named)
    if flag:
        return acreate_assistants(custom_llm_provider, **arguments)
    payload = create_payload(custom_llm_provider, arguments)
    return Assistant.model_validate(assistant_answer(session().create_assistants, payload))


async def acreate_assistants(
    custom_llm_provider: str, client: CallValue = None, **kwargs: CallValue,
) -> Assistant:
    arguments = named_arguments(kwargs, {"client": client, "async_create_assistants": True})
    payload = create_payload(custom_llm_provider, arguments)
    answer = await assistant_aanswer(session().acreate_assistants, payload)
    return Assistant.model_validate(answer)


def delete_assistant(
    custom_llm_provider: str, assistant_id: str, client: CallValue = None,
    api_key: CallValue = None, api_base: CallValue = None, api_version: CallValue = None,
    **kwargs: CallValue) -> DeletedAnswer:
    named = {"assistant_id": assistant_id, "client": client, "api_key": api_key,
             "api_base": api_base, "api_version": api_version}
    arguments = named_arguments(kwargs, named)
    match flag_argument("async_delete_assistants", kwargs, ValueError):
        case True:
            return adelete_assistant(custom_llm_provider, **arguments)
        case False:
            return read_delete_answer(AssistantDeleted, "delete", custom_llm_provider, arguments)


async def adelete_assistant(
    custom_llm_provider: str, client: CallValue = None, **kwargs: CallValue,
) -> AssistantDeleted:
    arguments = named_arguments(kwargs, {"client": client, "async_delete_assistants": True})
    payload = read_delete_payload("delete", custom_llm_provider, "async", arguments)
    answer = await assistant_aanswer(session().adelete_assistant, payload)
    return AssistantDeleted.model_validate(answer)


def create_thread(
    custom_llm_provider: str, messages: CallValue = None, metadata: CallValue = None,
    tool_resources: CallValue = None, client: CallValue = None, **kwargs: CallValue,
) -> Thread | Coroutine[None, None, Thread | None] | None:
    named = {
        "messages": messages, "metadata": metadata,
        "tool_resources": tool_resources, "client": client,
    }
    arguments = named_arguments(kwargs, named)
    if kwargs.get("acreate_thread") is True:
        return acreate_thread(custom_llm_provider, **arguments)
    payload = thread_payload(custom_llm_provider, None, arguments)
    return thread_of(assistant_answer(session().create_thread, payload))


async def acreate_thread(custom_llm_provider: str, **kwargs: CallValue) -> Thread | None:
    payload = thread_payload(custom_llm_provider, None, kwargs)
    return thread_of(await assistant_aanswer(session().acreate_thread, payload))


def get_thread(
    custom_llm_provider: str, thread_id: str, client: CallValue = None, **kwargs: CallValue,
) -> Thread | Coroutine[None, None, Thread | None] | None:
    arguments = named_arguments(kwargs, {"client": client})
    if kwargs.get("aget_thread") is True:
        return aget_thread(custom_llm_provider, thread_id, **arguments)
    payload = thread_payload(custom_llm_provider, thread_id, arguments)
    return thread_of(assistant_answer(session().get_thread, payload))


async def aget_thread(
    custom_llm_provider: str, thread_id: str, client: CallValue = None, **kwargs: CallValue,
) -> Thread | None:
    arguments = named_arguments(kwargs, {"client": client})
    payload = thread_payload(custom_llm_provider, thread_id, arguments)
    return thread_of(await assistant_aanswer(session().aget_thread, payload))
