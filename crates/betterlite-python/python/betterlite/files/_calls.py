import json
from collections.abc import Awaitable, Callable, Iterator, Mapping
from typing import Final, Literal, NamedTuple, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import acall_native, call_native
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeFileCallError, NativeFileCallSession
from betterlite.files._content import SETTING_NAMES, merged_headers, timeout_payload
from betterlite.types.llms.openai.core.aliases import FileTypes
from betterlite.types.llms.openai.core.files import (
    CREATE_FILE_REQUESTS_PURPOSE,
    FileExpiresAfter,
    FileListPage,
    OpenAIFileObject,
)
from betterlite.types.utils.base import ResponseObject

FileCreateProvider: TypeAlias = Literal[
    "openai", "azure", "gemini", "vertex_ai", "bedrock", "hosted_vllm", "litellm_proxy", "manus",
    "anthropic", "mistral", "xai"]
FileRetrieveProvider: TypeAlias = Literal[
    "openai", "azure", "gemini", "vertex_ai", "hosted_vllm", "litellm_proxy", "manus",
    "anthropic", "mistral", "xai"]
FileDeleteProvider: TypeAlias = Literal[
    "openai", "azure", "gemini", "vertex_ai", "bedrock", "litellm_proxy", "manus", "anthropic",
    "mistral", "xai"]
FileListProvider: TypeAlias = Literal[
    "openai", "azure", "litellm_proxy", "manus", "anthropic", "mistral", "xai"]

# the GenericLiteLLMParams fields every one of the four calls reads
PARAM_NAMES: Final[tuple[str, ...]] = (
    "api_key", "azure_ad_token", "tenant_id", "client_id", "client_secret", "azure_username",
    "azure_password", "azure_scope", "api_base", "api_version", "organization", "max_retries",
    "litellm_call_id", "id", "passthrough")
# the openai SDK names a bare bytes upload "upload" and a stream after its file
UPLOAD_NAME: Final = "upload"


class FileDeleted(ResponseObject):
    id: str
    object: Literal["file"]
    deleted: bool


class FilePage(FileListPage):
    # LiteLLM hands back the SDK's SyncCursorPage, whose iteration yields the files themselves
    @override
    def __iter__(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L765-L773
        self,
    ) -> Iterator[OpenAIFileObject]:
        return iter(self.data)


class CallArguments(NamedTuple):
    file_id: str | None
    model: str | None
    custom_llm_provider: str | None
    purpose: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None


FILE: Final[TypeAdapter[OpenAIFileObject]] = TypeAdapter(OpenAIFileObject)
DELETED: Final[TypeAdapter[FileDeleted]] = TypeAdapter(FileDeleted)
PAGE: Final[TypeAdapter[FilePage]] = TypeAdapter(FilePage)
MODEL: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)

T = TypeVar("T")
SyncAnswer: TypeAlias = Callable[[CallArguments, Mapping[str, CallValue]], T]
AsyncAnswer: TypeAlias = Callable[[CallArguments, Mapping[str, CallValue]], Awaitable[T]]


def file_call_session() -> NativeFileCallSession:
    settings: dict[str, JsonValue] = {"settings": read_settings(SETTING_NAMES)}
    return NativeFileCallSession(json.dumps(settings))


def call_values(
    arguments: CallArguments,
    options: Mapping[str, CallValue],
    **given: CallValue,
) -> CallPayload:
    if options.get("azure_ad_token_provider") is not None:
        raise UnsupportedCallArgument("azure_ad_token_provider")
    params: dict[str, CallValue] = {name: options.get(name) for name in PARAM_NAMES}
    params["timeout"] = timeout_payload(options.get("timeout"))
    params["request_timeout"] = timeout_payload(options.get("request_timeout"))
    values: dict[str, CallValue] = dict(given)
    values["custom_llm_provider"] = arguments.custom_llm_provider
    values["extra_headers"] = merged_headers(arguments.extra_headers)
    values["extra_body"] = arguments.extra_body
    values["params"] = params
    values["client"] = options.get("client")
    values["shared_session"] = options.get("shared_session")
    values["logger_fn"] = options.get("logger_fn")
    return to_call_payload(values)


def create_values(
    file: FileTypes,
    purpose: CREATE_FILE_REQUESTS_PURPOSE,
    expires_after: FileExpiresAfter | None,
    arguments: CallArguments,
    options: Mapping[str, CallValue],
) -> CallPayload:
    expires: dict[str, CallValue] | None = None
    if expires_after is not None:
        expires = {"anchor": expires_after["anchor"], "seconds": expires_after["seconds"]}
    wrapped: CallValue
    match file:
        case bytes():
            wrapped = (UPLOAD_NAME, file)
        case object():
            wrapped = file
    return call_values(arguments, options, file=wrapped, purpose=purpose, expires_after=expires)


async def async_create(
    file: FileTypes,
    purpose: CREATE_FILE_REQUESTS_PURPOSE,
    expires_after: FileExpiresAfter | None,
    arguments: CallArguments,
    options: Mapping[str, CallValue],
) -> OpenAIFileObject:
    payload = create_values(file, purpose, expires_after, arguments, options)
    answer = await acall_native(file_call_session().acreate, payload, NativeFileCallError)
    return FILE.validate_python(answer)


async def async_delete(
    arguments: CallArguments, options: Mapping[str, CallValue]
) -> FileDeleted:
    payload = call_values(arguments, options, file_id=arguments.file_id, model=arguments.model)
    answer = await acall_native(file_call_session().adelete, payload, NativeFileCallError)
    return DELETED.validate_python(answer)


async def async_retrieve(
    arguments: CallArguments, options: Mapping[str, CallValue]
) -> OpenAIFileObject:
    payload = call_values(arguments, options, file_id=arguments.file_id)
    answer = await acall_native(file_call_session().aretrieve, payload, NativeFileCallError)
    return FILE.validate_python(answer)


async def async_list(arguments: CallArguments, options: Mapping[str, CallValue]) -> FilePage:
    payload = call_values(arguments, options, purpose=arguments.purpose)
    answer = await acall_native(file_call_session().alist, payload, NativeFileCallError)
    return PAGE.validate_python(answer)


def dispatched(
    arguments: CallArguments,
    options: dict[str, CallValue],
    flag: str,
    asynchronous: AsyncAnswer[T],
    synchronous: SyncAnswer[T],
) -> T | Awaitable[T]:
    flagged = options.pop(flag, False) is True
    if flagged or options.pop("is_async", False) is True:
        return asynchronous(arguments, options)
    return synchronous(arguments, options)


def sync_retrieve(arguments: CallArguments, options: Mapping[str, CallValue]) -> OpenAIFileObject:
    payload = call_values(arguments, options, file_id=arguments.file_id)
    answer = call_native(file_call_session().retrieve, payload, NativeFileCallError)
    return FILE.validate_python(answer)


def sync_delete(arguments: CallArguments, options: Mapping[str, CallValue]) -> FileDeleted:
    payload = call_values(arguments, options, file_id=arguments.file_id, model=arguments.model)
    answer = call_native(file_call_session().delete, payload, NativeFileCallError)
    return DELETED.validate_python(answer)


def sync_list(arguments: CallArguments, options: Mapping[str, CallValue]) -> FilePage:
    payload = call_values(arguments, options, purpose=arguments.purpose)
    answer = call_native(file_call_session().list, payload, NativeFileCallError)
    return PAGE.validate_python(answer)


def create_file(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L161-L169
    file: FileTypes,
    purpose: CREATE_FILE_REQUESTS_PURPOSE,
    expires_after: FileExpiresAfter | None = None,
    custom_llm_provider: FileCreateProvider | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> OpenAIFileObject | Awaitable[OpenAIFileObject]:
    arguments = CallArguments(None, None, custom_llm_provider, None, extra_headers, extra_body)
    if kwargs.pop("acreate_file", False) is True:
        return async_create(file, purpose, expires_after, arguments, kwargs)
    payload = create_values(file, purpose, expires_after, arguments, kwargs)
    answer = call_native(file_call_session().create, payload, NativeFileCallError)
    return FILE.validate_python(answer)


async def acreate_file(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L116-L124
    file: FileTypes,
    purpose: CREATE_FILE_REQUESTS_PURPOSE,
    expires_after: FileExpiresAfter | None = None,
    custom_llm_provider: FileCreateProvider = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> OpenAIFileObject:
    arguments = CallArguments(None, None, custom_llm_provider, None, extra_headers, extra_body)
    return await async_create(file, purpose, expires_after, arguments, kwargs)


def file_retrieve(
    file_id: str, custom_llm_provider: FileRetrieveProvider = "openai",
    extra_headers: dict[str, str] | None = None, extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> OpenAIFileObject | Awaitable[OpenAIFileObject]:
    arguments = CallArguments(file_id, None, custom_llm_provider, None, extra_headers, extra_body)
    return dispatched(arguments, kwargs, "afile_retrieve", async_retrieve, sync_retrieve)


async def afile_retrieve(
    file_id: str, custom_llm_provider: FileRetrieveProvider = "openai",
    extra_headers: dict[str, str] | None = None, extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> OpenAIFileObject:
    arguments = CallArguments(file_id, None, custom_llm_provider, None, extra_headers, extra_body)
    return await async_retrieve(arguments, kwargs)


def file_delete(
    file_id: str, model: str | None = None,
    custom_llm_provider: FileDeleteProvider | str = "openai",
    extra_headers: dict[str, str] | None = None, extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> FileDeleted | Awaitable[FileDeleted]:
    arguments = CallArguments(file_id, model, custom_llm_provider, None, extra_headers, extra_body)
    return dispatched(arguments, kwargs, "afile_delete", async_delete, sync_delete)


async def afile_delete(
    file_id: str,
    custom_llm_provider: FileDeleteProvider = "openai",
    extra_headers: dict[str, str] | None = None,
    extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> FileDeleted:
    model = MODEL.validate_python(kwargs.pop("model", None))
    arguments = CallArguments(file_id, model, custom_llm_provider, None, extra_headers, extra_body)
    return await async_delete(arguments, kwargs)


def file_list(
    custom_llm_provider: FileListProvider = "openai", purpose: str | None = None,
    extra_headers: dict[str, str] | None = None, extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> FilePage | Awaitable[FilePage]:
    arguments = CallArguments(None, None, custom_llm_provider, purpose, extra_headers, extra_body)
    return dispatched(arguments, kwargs, "afile_list", async_list, sync_list)


async def afile_list(
    custom_llm_provider: FileListProvider = "openai", purpose: str | None = None,
    extra_headers: dict[str, str] | None = None, extra_body: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> FilePage:
    arguments = CallArguments(None, None, custom_llm_provider, purpose, extra_headers, extra_body)
    return await async_list(arguments, kwargs)
