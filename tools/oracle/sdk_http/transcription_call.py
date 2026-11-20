import asyncio
from typing import Final, Literal

import litellm
import openai
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from openai_chat import status_code

DURATION: Final = TypeAdapter(float | None)


class UnexpectedResponseError(TypeError):
    def __init__(self) -> None:
        super().__init__("litellm returned no TranscriptionResponse")


class File(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filename: str
    content: str


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Literal["sync", "async"]
    model: str | None = None
    missing_model: bool = False
    file: File
    language: str | None = None
    prompt: str | None = None
    response_format: Literal["json", "text", "srt", "verbose_json", "vtt"] | None = None
    timestamp_granularities: list[Literal["word", "segment"]] | None = None
    temperature: int | None = None
    user: str | None = None
    api_key: str | None = None
    api_version: str | None = None
    timeout_seconds: float | None = None
    max_retries: int | None = None
    custom_llm_provider: str | None = None
    base_url: str | None = None
    azure_ad_token: str | None = None
    extra_headers: dict[str, str] | None = None
    params: dict[str, JsonValue] = Field(default_factory=dict)


class Transcribed(BaseModel):
    model_config = ConfigDict(extra="ignore")

    text: str | None = None
    words: JsonValue = None
    duration: float | None = None


TRANSCRIBED: Final = TypeAdapter(Transcribed)


class Raised(BaseModel):
    model_config = ConfigDict(extra="forbid")

    cls: str = Field(serialization_alias="class")
    message: str


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    text: str | None = None
    words: JsonValue = None
    public_duration: float | None = None
    hidden_duration: float | None = None
    status: int | None = None
    raised: Raised | None = None


class TranscriptionCallValue(BaseModel):
    value: Outcome


class Result(BaseModel):
    transcription_call: TranscriptionCallValue


class TranscriptionCallRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    transcription_call: Call

    def serialize(self, api_base: str) -> str:
        return Result(
            transcription_call=TranscriptionCallValue(
                value=run(self.transcription_call, api_base)
            )
        ).model_dump_json(by_alias=True)


def run(call: Call, api_base: str) -> Outcome:
    file: FileTypes = (call.file.filename, call.file.content.encode(), "audio/wav")
    try:
        response = (
            asyncio.run(async_transcription(call, file, api_base))
            if call.mode == "async"
            else sync_transcription(call, file, api_base)
        )
    except KeyError as error:
        return Outcome(raised=Raised(cls="KeyError", message=str(error)))
    except ValueError as error:
        return Outcome(raised=Raised(cls="ValueError", message=str(error)))
    except openai.APIError as error:
        return Outcome(status=status_code(error))
    return project(response)


def sync_transcription(
    call: Call, file: FileTypes, api_base: str
) -> TranscriptionResponse:
    response = litellm.transcription(
        model=call.model or "",
        file=file,
        api_base=api_base,
        api_key=call.api_key,
        base_url=call.base_url,
        api_version=call.api_version,
        language=call.language,
        prompt=call.prompt,
        response_format=call.response_format,
        timestamp_granularities=call.timestamp_granularities,
        temperature=call.temperature,
        user=call.user,
        timeout=call.timeout_seconds,
        max_retries=call.max_retries,
        custom_llm_provider=call.custom_llm_provider,
        extra_headers=call.extra_headers,
        azure_ad_token=call.azure_ad_token,
        **call.params,
    )
    if not isinstance(response, TranscriptionResponse):
        raise UnexpectedResponseError
    return response


async def async_transcription(
    call: Call, file: FileTypes, api_base: str
) -> TranscriptionResponse:
    values: dict[str, object] = {
        "file": file,
        "api_base": api_base,
        "api_key": call.api_key,
        "timeout": call.timeout_seconds,
        "max_retries": call.max_retries,
        "extra_headers": call.extra_headers,
        **call.params,
    }
    if not call.missing_model:
        values["model"] = call.model or ""
    return await litellm.atranscription(**values)


def project(response: TranscriptionResponse) -> Outcome:
    transcribed = TRANSCRIBED.validate_json(response.model_dump_json())
    hidden_duration: float | None = DURATION.validate_python(response._hidden_params.get("audio_transcription_duration"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2748
    return Outcome(
        text=transcribed.text,
        words=transcribed.words,
        public_duration=transcribed.duration,
        hidden_duration=hidden_duration,
    )
