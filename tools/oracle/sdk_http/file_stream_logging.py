import asyncio
import datetime as dt
from collections.abc import AsyncIterator, Iterator
from typing import Final, Literal, Protocol

from litellm.files.streaming import FileContentStreamingResponse
from litellm.files.types import FileContentStreamingResult
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.files.transformation import BaseFilesConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import FileContentRequest
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, JsonValue
from typing_extensions import override

Scenario = Literal["drain", "fail", "close"]
FAIL_AFTER: Final = 1


class FileStreamLoggingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_id: str
    model: str
    scenario: Scenario
    chunk_size: int


class Outcome(BaseModel):
    chunks: list[list[int]]
    raised: bool
    success_calls: int
    failure_calls: int
    after_close_chunks: int
    capability: str | None


class FileStreamLoggingEnvelope(BaseModel):
    file_stream_logging: dict[str, Outcome]


class FilesHandler(Protocol):
    async def async_retrieve_file_content_streaming(  # ast-grep-ignore: no-long-names
        self,
        file_content_request: FileContentRequest,
        provider_config: BaseFilesConfig,
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
        logging_obj: Logging,
        chunk_size: int,
    ) -> FileContentStreamingResult: ...


HANDLER: Final[FilesHandler] = BaseLLMHTTPHandler()


class Capture(CustomLogger):
    success: list[dict[str, JsonValue]]
    failure: list[dict[str, JsonValue]]

    def __init__(self) -> None:
        super().__init__()
        self.success = []
        self.failure = []

    @override
    async def async_log_success_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: dt.datetime | None,
        end_time: dt.datetime | None,
    ) -> None:
        del response_obj, start_time, end_time
        self.success.append(kwargs)

    @override
    async def async_log_failure_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: dt.datetime | None,
        end_time: dt.datetime | None,
    ) -> None:
        del response_obj, start_time, end_time
        self.failure.append(kwargs)


class FaultyStream:
    def __init__(self, inner: AsyncIterator[bytes], fail_after: int | None) -> None:
        self.inner = inner
        self.fail_after = fail_after
        self.served = 0

    def __aiter__(self) -> "FaultyStream":
        return self

    async def __anext__(self) -> bytes:
        if self.fail_after is not None and self.served >= self.fail_after:
            raise ValueError("broken")
        chunk = await self.inner.__anext__()
        self.served += 1
        return chunk


def provider_config() -> BaseFilesConfig:
    config = ProviderConfigManager.get_provider_files_config("", LlmProviders.MANUS)
    if config is None:
        raise TypeError("LiteLLM has no files config for manus")
    return config


def logging_fixture(capture: Capture) -> Logging:
    return Logging(
        model="base",
        messages=None,
        stream=False,
        call_type="afile_content",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="file-stream-logging",
        function_id="file-stream-logging",
        dynamic_async_success_callbacks=[capture],
        dynamic_async_failure_callbacks=[capture],
    )


def async_stream(stream: Iterator[bytes] | AsyncIterator[bytes]) -> AsyncIterator[bytes]:
    match stream:
        case AsyncIterator():
            return stream
        case Iterator():
            raise TypeError("the LiteLLM file stream is not async")


async def run_case(call: FileStreamLoggingCall, api_base: str) -> Outcome:
    capture = Capture()
    logging_obj = logging_fixture(capture)
    result = await HANDLER.async_retrieve_file_content_streaming(
        file_content_request=FileContentRequest(file_id=call.file_id),
        provider_config=provider_config(),
        litellm_params={"api_base": api_base},
        headers={},
        logging_obj=logging_obj,
        chunk_size=call.chunk_size,
    )
    fail_after = FAIL_AFTER if call.scenario == "fail" else None
    response = FileContentStreamingResponse(
        FaultyStream(async_stream(result.stream_iterator), fail_after),
        call.file_id,
        call.model,
        None,
        logging_obj,
    )
    outcome = Outcome(
        chunks=[],
        raised=False,
        success_calls=0,
        failure_calls=0,
        after_close_chunks=0,
        capability=None,
    )
    if call.scenario == "close":
        outcome.chunks.append(list(await response.__anext__()))
        response.close()
        outcome.after_close_chunks = len(list(response))
        try:
            response.__aiter__()
        except TypeError as error:
            outcome.capability = str(error)
    else:
        iterator = response.__aiter__()
        while True:
            try:
                chunk = await iterator.__anext__()
            except StopAsyncIteration:
                break
            except ValueError:
                outcome.raised = True
                break
            outcome.chunks.append(list(chunk))
    outcome.success_calls = len(capture.success)
    outcome.failure_calls = len(capture.failure)
    return outcome


class FileStreamLoggingRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_stream_logging: FileStreamLoggingCall

    def serialize(self, api_base: str) -> str:
        outcome = asyncio.run(run_case(self.file_stream_logging, api_base))
        envelope = FileStreamLoggingEnvelope(file_stream_logging={"value": outcome})
        return envelope.model_dump_json()
