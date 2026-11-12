import asyncio
from collections.abc import AsyncGenerator, AsyncIterator, Iterator, Mapping
from typing import Literal, TypeAlias, TypedDict, cast

import httpx
from litellm.files.utils import FilesAPIUtils
from litellm.llms.base_llm.chat.transformation import BaseConfig
from litellm.llms.base_llm.files.transformation import (
    BaseFilesConfig,
    BaseFileUploadStream,
    LiteLLMLoggingObj,
)
from litellm.llms.mistral.files.transformation import MistralFilesConfig
from litellm.types.utils import ExtractedFileData
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
MISTRAL_URL = "https://mock.invalid/v1/files"


class LazyFileSpec(TypedDict):
    lazy: bool


class Case(TypedDict, total=False):
    purpose: str | None
    content_type: str | None
    content: str | None
    file: str | LazyFileSpec | None


class BatchExtractedInput(TypedDict):
    function: Literal["batch_extracted_predicate"]
    cases: list[Case]


class BatchMetadataInput(TypedDict):
    function: Literal["batch_metadata_predicate"]
    cases: list[Case]


class BaseDefaultsInput(TypedDict):
    function: Literal["base_defaults"]
    response_body: str
    optional_params: Object
    litellm_params: Object


class FileUrlInput(TypedDict):
    function: Literal["file_url_forwarding"]
    api_base: str
    api_key: str
    model: str
    optional_params: Object
    litellm_params: Object
    data: Object


class StreamPassthroughInput(TypedDict):
    function: Literal["stream_passthrough_and_replay"]
    chunks_hex: list[str]
    headers: dict[str, str]
    request_url: str
    litellm_params: Object
    upload_chunks_hex: list[str]


Input = (
    BatchExtractedInput
    | BatchMetadataInput
    | BaseDefaultsInput
    | FileUrlInput
    | StreamPassthroughInput
)


class RecordingFilesConfig(MistralFilesConfig):
    calls: list[Json]

    def __init__(self) -> None:
        self.calls = []

    @override
    def get_complete_url(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L123-L133
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L123-L133
        litellm_params: dict[str, Json],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L123-L133
        stream: bool | None = None,
    ) -> str:
        self.calls.append(
            {
                "api_base": api_base,
                "api_key": api_key,
                "model": model,
                "optional_params": optional_params,
                "litellm_params": litellm_params,
            }
        )
        return MISTRAL_URL


class DefaultFilesConfig(MistralFilesConfig):
    def __init__(self) -> None:
        pass

    @override
    def get_complete_url(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L123-L133
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L123-L133
        litellm_params: dict[str, Json],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L123-L133
        stream: bool | None = None,
    ) -> str:
        return BaseConfig.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L292-L310
            self,
            api_base=api_base,
            api_key=api_key,
            model=model,
            optional_params=optional_params,
            litellm_params=litellm_params,
            stream=stream,
        )


class LazyFile:
    def read(self) -> bytes:
        raise AssertionError(
            "the batch predicate read the file"
        )


class UploadStream(BaseFileUploadStream):
    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks

    @override
    def iter_bytes(self) -> Iterator[bytes]:
        return iter(self.chunks)


def _extracted_predicate(case: Case) -> bool:
    content = case.get("content")
    headers: dict[str, str] = {}
    extracted = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4392-L4406
        "ExtractedFileData",
        {
            "filename": None,
            "content": None if content is None else content.encode(),
            "content_type": case.get("content_type"),
            "headers": headers,
        },
    )
    return FilesAPIUtils.is_batch_jsonl_file(
        {"purpose": case.get("purpose")},  # type: ignore[typeddict-item, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472-L493
        extracted,
    )


def _metadata_predicate(case: Case) -> bool:
    file_value = case.get("file")
    file_object = None if file_value is None else LazyFile()
    return FilesAPIUtils.is_batch_jsonl_request(
        {"purpose": case.get("purpose"), "file": file_object},  # type: ignore[typeddict-item, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472-L493
        case.get("content_type"),
    )


def _extracted_predicates(cases: list[Case]) -> Json:
    return [
        _extracted_predicate(case)
        for case in cases
    ]


def _metadata_predicates(cases: list[Case]) -> Json:
    return [
        _metadata_predicate(case)
        for case in cases
    ]


def _base_defaults(input_value: BaseDefaultsInput) -> Json:
    config = DefaultFilesConfig()
    response = httpx.Response(
        status_code=200,
        content=input_value["response_body"].encode(),
        request=httpx.Request("GET", MISTRAL_URL),
    )
    next_request = config.transform_list_files_next_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L163-L177
        raw_response=response,
        optional_params=input_value["optional_params"],
        litellm_params=input_value["litellm_params"],
    )
    if next_request is not None:
        raise ValueError("the base files config returned a next page request")
    return {"http_method": config.file_upload_http_method, "next_request": None}


def _forwarded_url(
    config: MistralFilesConfig,
    input_value: FileUrlInput,
    api_key: str,
    api_base: str | None,
) -> str:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L74-L89
        "str",
        config.get_complete_file_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L74-L89
            api_base,
            api_key,
            input_value["model"],
            input_value["optional_params"],
            input_value["litellm_params"],
            input_value["data"],  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L472-L493
        ),
    )


def _file_url_forwarding(input_value: FileUrlInput) -> Json:
    config = RecordingFilesConfig()
    api_key = input_value["api_key"]
    url = _forwarded_url(config, input_value, api_key, input_value["api_base"])
    missing: Json = None
    try:
        _forwarded_url(DefaultFilesConfig(), input_value, api_key, None)
    except ValueError as error:
        missing = str(error)
    return {"url": url, "calls": config.calls, "missing_api_base": missing}


async def _chunks(chunks: list[bytes]) -> AsyncGenerator[bytes, None]:
    for chunk in chunks:
        yield chunk


async def _collect_stream(
    config: BaseFilesConfig,
    stream: AsyncGenerator[bytes, None],
    headers: dict[str, str],
    request_url: str,
    litellm_params: Object,
) -> tuple[list[bytes], Mapping[str, str]]:
    result = await config.transform_file_content_stream(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L199-L209
        stream_iterator=stream,
        headers=headers,
        request_url=request_url,
        logging_obj=cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L199-L209
            "LiteLLMLoggingObj", None
        ),
        litellm_params=litellm_params,
    )
    collected = [
        chunk
        async for chunk in cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/types.py#L8-L10
            "AsyncIterator[bytes]", result.stream_iterator
        )
    ]
    return collected, result.headers


def _stream_passthrough(input_value: StreamPassthroughInput) -> Json:
    config = DefaultFilesConfig()
    collected, headers = asyncio.run(
        _collect_stream(
            config,
            _chunks([bytes.fromhex(chunk) for chunk in input_value["chunks_hex"]]),
            input_value["headers"],
            input_value["request_url"],
            input_value["litellm_params"],
        )
    )
    upload = UploadStream([bytes.fromhex(chunk) for chunk in input_value["upload_chunks_hex"]])
    streamed: list[Json] = [chunk.hex() for chunk in collected]
    first: list[Json] = [chunk.hex() for chunk in upload.iter_bytes()]
    second: list[Json] = [chunk.hex() for chunk in upload.iter_bytes()]
    return {
        "stream": {"chunks": streamed, "headers": dict(headers)},
        "replay": {"first": first, "second": second},
    }


def run(input_value: Input) -> Json:
    if input_value["function"] == "batch_extracted_predicate":
        return _extracted_predicates(input_value["cases"])
    if input_value["function"] == "batch_metadata_predicate":
        return _metadata_predicates(input_value["cases"])
    if input_value["function"] == "base_defaults":
        return _base_defaults(input_value)
    if input_value["function"] == "file_url_forwarding":
        return _file_url_forwarding(input_value)
    return _stream_passthrough(input_value)
