import asyncio
import importlib
import inspect
from collections.abc import AsyncIterator, Awaitable, Coroutine, Iterator, Mapping
from typing import Final, Literal, Protocol, TypeAlias, cast, overload

import httpx
import openai
import pytest
from batch_server import parameter_rows
from file_server import (
    BODY,
    CONTENT_PATH,
    CONTENT_TYPE,
    FILE_ID,
    GOOD_KEY,
    JSON_BODY,
    JSON_ID,
    JSON_PATH,
    JSON_TYPE,
    MISSING_ID,
    NOT_FOUND,
    OK,
    UPSTREAM_HEADER,
    UPSTREAM_VALUE,
    FileServer,
    RecordedCall,
)
from pydantic import JsonValue
from typing_extensions import override

import betterlite
from betterlite._completion.errors import ERROR_PAYLOAD, UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue
from betterlite._endpoints.proxy import ProxyAuthProvider
from betterlite._native import NativeFileContentError
from betterlite.exceptions import APIConnectionError, NotFoundError
from betterlite.files._content import (
    FileCallArguments,
    FileContentOutcome,
    HttpxBinaryResponseContent,
    afile_content,
    call_payload_values,
    file_content,
)
from betterlite.files.streaming import FileContentStreamingResponse
from betterlite.files.types import FileContentStreamingResult
from betterlite.files.utils import ExtractedFileData, FilesAPIUtils
from betterlite.types.llms.openai.core.files import CreateFileRequest

CALLS_PER_COMPARISON = 2
SYNC_REFUSED = "File content stream does not support sync iteration"
ASYNC_REFUSED = "File content stream does not support async iteration"
JSON_OBJECT: Final[dict[str, JsonValue]] = {"ok": True}
CONTENT_TYPES: Final[tuple[str | None, ...]] = (
    "application/jsonl",
    "application/json",
    "application/octet-stream",
    "application/x-ndjson",
    "application/x-jsonlines",
    "text/plain",
    "application/pdf",
    None,
)
BATCH_FILE: Final[CreateFileRequest] = {"file": b"rows", "purpose": "batch"}
TUNED_FILE: Final[CreateFileRequest] = {"file": b"rows", "purpose": "fine-tune"}
EXTRACTED: Final[ExtractedFileData] = {
    "filename": "rows.jsonl",
    "content": b"{}\n",
    "content_type": "application/json",
    "headers": {},
}
Outcome: TypeAlias = FileContentOutcome | Awaitable[FileContentOutcome]


class ReferenceResponse(Protocol):
    status_code: int
    headers: Mapping[str, str]


class ReferencePlain(Protocol):
    response: ReferenceResponse
    content: bytes
    text: str

    def read(self) -> bytes: ...

    def json(self) -> JsonValue: ...


# LiteLLM's wrapper answers both iteration paths and refuses the one the provider did not give it
class ChunkStream(Protocol):
    _hidden_params: dict[str, JsonValue]
    custom_llm_provider: str | None

    def __iter__(self) -> Iterator[bytes]: ...

    def __next__(self) -> bytes: ...

    def __aiter__(self) -> AsyncIterator[bytes]: ...

    def __anext__(self) -> Awaitable[bytes]: ...

    def close(self) -> None: ...

    def aclose(self) -> Awaitable[None]: ...


class ReferenceStream(Protocol):
    stream_iterator: ChunkStream
    headers: Mapping[str, str]


class LiteLLMFiles(Protocol):
    @overload
    def file_content(
        self, file_id: str, *, stream: Literal[True], **kwargs: CallValue
    ) -> ReferenceStream: ...

    @overload
    def file_content(
        self,
        file_id: str,
        *,
        afile_content: Literal[True],
        stream: Literal[False] = False,
        **kwargs: CallValue,
    ) -> Coroutine[None, None, ReferencePlain]: ...

    @overload
    def file_content(
        self,
        file_id: str,
        *,
        afile_content: Literal[False] = False,
        stream: Literal[False] = False,
        **kwargs: CallValue,
    ) -> ReferencePlain: ...

    def file_content(
        self, file_id: str, **kwargs: CallValue
    ) -> ReferencePlain | ReferenceStream | Coroutine[None, None, ReferencePlain]: ...

    @overload
    def afile_content(
        self, file_id: str, *, stream: Literal[True], **kwargs: CallValue
    ) -> Coroutine[None, None, ReferenceStream]: ...

    @overload
    def afile_content(
        self, file_id: str, *, stream: Literal[False] = False, **kwargs: CallValue
    ) -> Coroutine[None, None, ReferencePlain]: ...

    def afile_content(
        self, file_id: str, **kwargs: CallValue
    ) -> Coroutine[None, None, ReferencePlain | ReferenceStream]: ...


class ReferenceFileUtils(Protocol):
    def valid_content_type(self, content_type: str | None) -> bool: ...

    def is_batch_jsonl_request(
        self, create_file_data: CreateFileRequest, content_type: str | None
    ) -> bool: ...

    def is_batch_jsonl_file(
        self, create_file_data: CreateFileRequest, extracted_file_data: ExtractedFileData
    ) -> bool: ...


class FileUtilsModule(Protocol):
    FilesAPIUtils: ReferenceFileUtils


REFERENCE = cast("LiteLLMFiles", importlib.import_module("litellm"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L806-L860
UTILS_MODULE = cast("FileUtilsModule", importlib.import_module("litellm.files.utils"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/utils.py#L23-L45


def made_plain(outcome: Outcome) -> HttpxBinaryResponseContent:
    match outcome:
        case HttpxBinaryResponseContent():
            return outcome
        case object():
            raise AssertionError("the call did not answer with a plain response")


def made_stream(outcome: Outcome) -> FileContentStreamingResult:
    match outcome:
        case FileContentStreamingResult():
            return outcome
        case object():
            raise AssertionError("the call did not answer with a stream")


def wrapper_of(result: FileContentStreamingResult) -> FileContentStreamingResponse:
    iterator = result.stream_iterator
    match iterator:
        case FileContentStreamingResponse():
            return iterator
        case object():
            raise AssertionError("the stream iterator is not the streaming wrapper")


def sync_chunks(iterator: Iterator[bytes] | AsyncIterator[bytes]) -> list[bytes]:
    match iterator:
        case Iterator():
            return list(iterator)
        case object():
            raise AssertionError("the stream has no sync chunks")


async def async_chunks(iterator: Iterator[bytes] | AsyncIterator[bytes]) -> list[bytes]:
    match iterator:
        case AsyncIterator():
            return [chunk async for chunk in iterator]
        case object():
            raise AssertionError("the stream has no async chunks")


async def awaited_plain(pending: Outcome) -> HttpxBinaryResponseContent:
    match pending:
        case Awaitable():
            return made_plain(await pending)
        case object():
            raise AssertionError("the call answered with a plain result instead of a coroutine")


def authorization(call: RecordedCall) -> str | None:
    lowered = {name.lower(): value for name, value in call["headers"].items()}
    return lowered.get("authorization")


def native_cause(error: BaseException) -> NativeFileContentError:
    cause = error.__cause__
    match cause:
        case NativeFileContentError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(file_content)) == parameter_rows(
        inspect.signature(REFERENCE.file_content)
    )
    assert parameter_rows(inspect.signature(afile_content)) == parameter_rows(
        inspect.signature(REFERENCE.afile_content)
    )
    assert not inspect.iscoroutinefunction(file_content)
    assert inspect.iscoroutinefunction(afile_content)


def test_plain_matches_litellm() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            extra_headers={"x-caller": "file-test"},
            organization="org-test",
        )
        made = made_plain(
            file_content(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
                extra_headers={"x-caller": "file-test"},
                organization="org-test",
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.status_code == reference.response.status_code == OK
    assert made.content == reference.content == BODY
    assert made.text == reference.text
    assert made.read() == reference.read() == BODY
    assert dict(made.headers)[UPSTREAM_HEADER] == UPSTREAM_VALUE
    assert reference.response.headers[UPSTREAM_HEADER] == UPSTREAM_VALUE
    assert dict(made.headers)["content-type"] == CONTENT_TYPE
    assert reference.response.headers["content-type"] == CONTENT_TYPE
    assert ours_call["method"] == "GET"
    assert ours_call["path"] == reference_call["path"] == CONTENT_PATH
    assert authorization(ours_call) == authorization(reference_call) == f"Bearer {GOOD_KEY}"

    for name, value in (("x-caller", "file-test"), ("openai-organization", "org-test")):
        ours_headers = {key.lower(): val for key, val in ours_call["headers"].items()}
        ref_headers = {key.lower(): val for key, val in reference_call["headers"].items()}
        assert ours_headers[name] == ref_headers[name] == value


def test_plain_json_matches() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=JSON_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        made = made_plain(
            file_content(
                file_id=JSON_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.json() == reference.json() == JSON_OBJECT
    assert made.content == reference.content == JSON_BODY
    assert ours_call["path"] == reference_call["path"] == JSON_PATH
    assert dict(made.headers)["content-type"] == JSON_TYPE


def test_plain_coroutine_flag() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            afile_content=True,
        )
        flagged = file_content(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            afile_content=True,
        )
        assert inspect.isawaitable(flagged)
        direct = afile_content(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert inspect.isawaitable(direct)
        made_flagged = asyncio.run(awaited_plain(flagged))
        made_direct = made_plain(asyncio.run(direct))
        reference_plain = asyncio.run(reference)
    assert made_flagged.content == reference_plain.content == BODY
    assert made_direct.status_code == reference_plain.response.status_code == OK


def test_stream_matches_litellm() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            stream=True,
        )
        made = made_stream(
            file_content(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
                stream=True,
            )
        )
        reference_chunks = sync_chunks(reference.stream_iterator)
        made_chunks = sync_chunks(made.stream_iterator)
        wrapper = wrapper_of(made)
        assert wrapper.file_id == FILE_ID
        assert wrapper.model is None
        assert wrapper.custom_llm_provider == "openai"
        assert wrapper.standard_logging_object is None
        ours_hidden = wrapper._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/streaming.py#L133-L135
        reference_hidden = reference.stream_iterator._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/streaming.py#L133-L135
        wrapper.close()
        after_close = list(wrapper)
    assert made_chunks == reference_chunks == [BODY]
    assert dict(made.headers)[UPSTREAM_HEADER] == UPSTREAM_VALUE
    assert reference.headers[UPSTREAM_HEADER] == UPSTREAM_VALUE
    assert after_close == []
    assert ours_hidden["litellm_model_name"] == reference_hidden["litellm_model_name"] is None
    assert ours_hidden["response_cost"] is None
    assert reference_hidden["response_cost"] is None


def test_stream_chunks_by_size() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            chunk_size=3,
            stream=True,
        )
        made = made_stream(
            file_content(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
                chunk_size=3,
                stream=True,
            )
        )
        reference_chunks = sync_chunks(reference.stream_iterator)
        made_chunks = sync_chunks(made.stream_iterator)
    assert made_chunks == reference_chunks == [BODY[:3], BODY[3:6], BODY[6:]]


def test_stream_async_matches() -> None:
    with FileServer() as server:
        reference = asyncio.run(
            REFERENCE.afile_content(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
                stream=True,
            )
        )
        made = made_stream(
            asyncio.run(
                afile_content(
                    file_id=FILE_ID,
                    custom_llm_provider="openai",
                    api_base=server.url,
                    api_key=GOOD_KEY,
                    stream=True,
                )
            )
        )
        reference_chunks = asyncio.run(async_chunks(reference.stream_iterator))
        made_chunks = asyncio.run(async_chunks(made.stream_iterator))
        wrapper = wrapper_of(made)
        asyncio.run(wrapper.aclose())
        asyncio.run(wrapper.aclose())
    assert made_chunks == reference_chunks == [BODY]


def test_stream_close_twice() -> None:
    with FileServer() as server:
        made = made_stream(
            file_content(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
                stream=True,
            )
        )
        wrapper = wrapper_of(made)
        assert next(wrapper) == BODY
        wrapper.close()
        wrapper.close()
        assert list(wrapper) == []
        with pytest.raises(TypeError, match=ASYNC_REFUSED) as refused:
            asyncio.run(wrapper.__anext__())
    assert str(refused.value) == ASYNC_REFUSED


def test_stream_refuses_wrong_path() -> None:
    with FileServer() as server:
        sync_wrapper = wrapper_of(
            made_stream(
                file_content(
                    file_id=FILE_ID,
                    custom_llm_provider="openai",
                    api_base=server.url,
                    api_key=GOOD_KEY,
                    stream=True,
                )
            )
        )
        async_wrapper = wrapper_of(
            made_stream(
                asyncio.run(
                    afile_content(
                        file_id=FILE_ID,
                        custom_llm_provider="openai",
                        api_base=server.url,
                        api_key=GOOD_KEY,
                        stream=True,
                    )
                )
            )
        )
        with pytest.raises(TypeError, match=ASYNC_REFUSED) as iter_refused:
            sync_wrapper.__aiter__()
        with pytest.raises(TypeError, match=ASYNC_REFUSED) as next_refused:
            asyncio.run(sync_wrapper.__anext__())
        with pytest.raises(TypeError, match=SYNC_REFUSED) as iter_back:
            async_wrapper.__iter__()
        with pytest.raises(TypeError, match=SYNC_REFUSED) as next_back:
            async_wrapper.__next__()
    assert str(iter_refused.value) == ASYNC_REFUSED
    assert str(next_refused.value) == ASYNC_REFUSED
    assert str(iter_back.value) == SYNC_REFUSED
    assert str(next_back.value) == SYNC_REFUSED


def test_plain_utils_match_litellm() -> None:
    reference = UTILS_MODULE.FilesAPIUtils
    for content_type in CONTENT_TYPES:
        assert FilesAPIUtils.valid_content_type(content_type) == reference.valid_content_type(
            content_type
        )
        assert FilesAPIUtils.is_batch_jsonl_request(
            BATCH_FILE, content_type
        ) == reference.is_batch_jsonl_request(BATCH_FILE, content_type)
        assert not reference.is_batch_jsonl_request(TUNED_FILE, content_type)
        assert not FilesAPIUtils.is_batch_jsonl_request(TUNED_FILE, content_type)
    assert FilesAPIUtils.is_batch_jsonl_file(BATCH_FILE, EXTRACTED)
    assert reference.is_batch_jsonl_file(BATCH_FILE, EXTRACTED)
    assert not FilesAPIUtils.is_batch_jsonl_file(TUNED_FILE, EXTRACTED)
    assert not reference.is_batch_jsonl_file(TUNED_FILE, EXTRACTED)


def test_plain_empty_batch_files() -> None:
    empty: CreateFileRequest = {"file": b"", "purpose": "batch"}
    extracted: ExtractedFileData = {**EXTRACTED, "content": b""}
    reference = UTILS_MODULE.FilesAPIUtils
    assert FilesAPIUtils.is_batch_jsonl_request(empty, "application/json")
    assert reference.is_batch_jsonl_request(empty, "application/json")
    assert FilesAPIUtils.is_batch_jsonl_file(empty, extracted)
    assert reference.is_batch_jsonl_file(empty, extracted)


def test_error_matches_litellm() -> None:
    with FileServer() as server:
        with pytest.raises(openai.NotFoundError) as reference:
            REFERENCE.file_content(
                file_id=MISSING_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(NotFoundError) as made:
            file_content(
                file_id=MISSING_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        assert len(server.calls) == CALLS_PER_COMPARISON
    assert made.value.status_code == reference.value.status_code == NOT_FOUND
    cause = native_cause(made.value)
    assert cause is made.value.__cause__

    expected = {
        "code": reference.value.code,
        "type": reference.value.type,
        "param": reference.value.param,
    }
    assert expected == {
        "code": "file_not_found", "type": "invalid_request_error", "param": "file_id",
    }
    assert ERROR_PAYLOAD.validate_json(str(cause)).get("body") == expected
    # B7870 records the shared exception converter's dropped fields as a deviation.
    assert made.value.code is made.value.type is made.value.param is None


def test_plain_azure_token() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=FILE_ID, custom_llm_provider="azure", api_base=server.url,
            api_version="2024-05-01-preview", azure_ad_token=GOOD_KEY,
        )
        made = made_plain(file_content(
            file_id=FILE_ID, custom_llm_provider="azure", api_base=server.url,
            api_version="2024-05-01-preview", azure_ad_token=GOOD_KEY,
        ))
        reference_call, ours_call = server.calls
    assert made.content == reference.content == BODY
    assert authorization(ours_call) == authorization(reference_call) == f"Bearer {GOOD_KEY}"


def test_stream_model_provider() -> None:
    with FileServer() as server:
        reference = REFERENCE.file_content(
            file_id=FILE_ID, model="gpt-4o", api_base=server.url,
            api_key=GOOD_KEY, stream=True,
        )
        made = made_stream(file_content(
            file_id=FILE_ID, model="gpt-4o", api_base=server.url,
            api_key=GOOD_KEY, stream=True,
        ))
        assert wrapper_of(made).custom_llm_provider == (
            reference.stream_iterator.custom_llm_provider
        ) == "openai"
        assert sync_chunks(made.stream_iterator) == sync_chunks(reference.stream_iterator)


def test_error_token_provider() -> None:
    with pytest.raises(UnsupportedCallArgument, match="azure_ad_token_provider"):
        file_content(file_id=FILE_ID, azure_ad_token_provider=GOOD_KEY)


def test_error_timeout_phase() -> None:
    timeout = httpx.Timeout(connect=1, read=0.05, write=1, pool=1)
    with FileServer(body_delay=0.2) as server:
        with pytest.raises(openai.APITimeoutError):
            REFERENCE.file_content(
                file_id=FILE_ID, custom_llm_provider="openai", api_base=server.url,
                api_key=GOOD_KEY, timeout=timeout, max_retries=0,
            )
        # B7870 records the shared exception converter's timeout class as a deviation.
        with pytest.raises(APIConnectionError) as made:
            file_content(
                file_id=FILE_ID, custom_llm_provider="openai", api_base=server.url,
                api_key=GOOD_KEY, timeout=timeout, max_retries=0,
            )
    assert made.value.status_code == httpx.codes.INTERNAL_SERVER_ERROR
    assert ERROR_PAYLOAD.validate_json(str(native_cause(made.value)))["type"] == (
        "APIConnectionError"
    )


class ProxyHeaders(ProxyAuthProvider):
    @override
    def get_auth_headers(self) -> Mapping[str, str]:
        return {"x-proxy": "proxy", "x-conflict": "proxy"}


@pytest.fixture(autouse=False)
def proxy_auth() -> Iterator[None]:
    previous = betterlite.proxy_auth
    betterlite.proxy_auth = ProxyHeaders()
    try:
        yield
    finally:
        betterlite.proxy_auth = previous


@pytest.mark.parametrize(("field", "value"), [
    ("api_key", GOOD_KEY), ("api_base", "https://example.test"), ("api_version", "v1"),
    ("organization", "org-test"), ("request_timeout", 4), ("max_retries", 0),
    ("litellm_call_id", "call-1"), ("id", "id-1"), ("azure_ad_token", GOOD_KEY),
    ("tenant_id", "tenant"), ("client_id", "client"), ("client_secret", GOOD_KEY),
    ("azure_username", "user"), ("azure_password", GOOD_KEY), ("azure_scope", "scope"),
])
def test_plain_payload_forwarding(field: str, value: CallValue) -> None:
    arguments = FileCallArguments(FILE_ID, None, "azure", None, None, 1024, stream=False)
    made = call_payload_values(arguments, {field: value})
    assert made[field] == value


@pytest.mark.usefixtures("proxy_auth")
def test_plain_proxy_and_timeout() -> None:
    arguments = FileCallArguments(
        FILE_ID, None, "openai", {"x-caller": "caller", "x-conflict": "caller"}, None,
        1024, stream=False,
    )
    made = call_payload_values(arguments, {
        "timeout": httpx.Timeout(connect=1, read=2, write=3, pool=4),
    })
    assert made["extra_headers"] == {
        "x-caller": "caller", "x-conflict": "proxy", "x-proxy": "proxy",
    }
    assert made["timeout"] == {"connect": 1, "read": 2, "write": 3, "pool": 4}
