import asyncio
from collections.abc import Coroutine
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, Unpack, runtime_checkable

import httpx
import litellm
import openai
from litellm.exceptions import BadRequestError
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from pydantic import BaseModel, ConfigDict, Field, JsonValue

from fine_tuning_read import JSON, Failure, StructuredTimeout, api_failure, timeout_value

# an unannotated exception class carries Any, so both clauses catch theirs through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
BAD_REQUEST: Final[type[openai.BadRequestError]] = BadRequestError

# the sync calls answer a record and the async ones a coroutine of one
WriteResult: TypeAlias = BaseModel | Coroutine[object, object, BaseModel]

Purpose: TypeAlias = Literal["assistants", "batch", "fine-tune", "messages"]


class ExpiresAfter(TypedDict):
    anchor: Literal["created_at"]
    seconds: int


# every keyword argument the four calls receive here; an absent key takes the function's default
class WriteValues(TypedDict, total=False):
    file: tuple[str, bytes, str]
    purpose: Purpose
    expires_after: ExpiresAfter | None
    custom_llm_provider: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    file_id: str
    model: str | None
    api_base: str
    api_key: str | None
    api_version: str | None
    organization: str | None
    timeout: float | str | httpx.Timeout | None
    request_timeout: float | str | None
    max_retries: int | None
    passthrough: bool | None


class WriteCall(Protocol):
    def __call__(self, **values: Unpack[WriteValues]) -> WriteResult: ...


# litellm's decorator leaves the four writes untyped, so each module name answers one typed call
class FilesApi(Protocol):
    def __getattr__(self, name: str) -> WriteCall: ...


API: FilesApi = litellm


class Params(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | StructuredTimeout | None = None
    request_timeout: float | str | None = None
    max_retries: int | None = None
    passthrough: bool | None = None


class FileArg(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filename: str
    content_type: str
    content: str


class ExpiresArg(BaseModel):
    model_config = ConfigDict(extra="forbid")

    anchor: Literal["created_at"]
    seconds: int


class UploadStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file: FileArg
    purpose: Purpose
    expires_after: ExpiresArg | None = None
    custom_llm_provider: str | None = None
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    params: Params = Field(default_factory=Params)


class DeleteStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_id: str
    model: str | None = None
    custom_llm_provider: str | None = None
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    params: Params = Field(default_factory=Params)


class CreateKind(BaseModel):
    create_file: UploadStep


class ACreateKind(BaseModel):
    acreate_file: UploadStep


class DeleteKind(BaseModel):
    file_delete: DeleteStep


class ADeleteKind(BaseModel):
    afile_delete: DeleteStep


Step = CreateKind | ACreateKind | DeleteKind | ADeleteKind


# the synthesized response and request a LiteLLM BadRequestError carries
class Rejection(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    llm_provider: str
    body: str
    method: str
    url: str


# BadRequestError leaves model and llm_provider unannotated, so pydantic reads them off the instance
class Raised(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    model: str
    llm_provider: str


# BaseLLMException leaves status_code and message unannotated too
class ProviderFailure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int
    message: str


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: JsonValue | None = None
    failure: Failure | None = None
    bad_request: Rejection | None = None


class FileWriteCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[Step]


class Result(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_provider_write: dict[str, dict[str, list[StepOutcome]]]


class FileProviderWriteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_provider_write: FileWriteCall

    def serialize(self, api_base: str) -> str:
        steps = [run_step(step, api_base) for step in self.file_provider_write.steps]
        result = Result(file_provider_write={"value": {"steps": steps}})
        return result.model_dump_json(by_alias=True, exclude_unset=True)


# the harness base stands in for an absent api_base
def write_values(step: UploadStep | DeleteStep, harness: str) -> WriteValues:
    params = step.params
    values = WriteValues(
        api_base=params.api_base if params.api_base is not None else harness,
        api_key=params.api_key,
        api_version=params.api_version,
        organization=params.organization,
        timeout=timeout_value(params.timeout),
        request_timeout=params.request_timeout,
        max_retries=params.max_retries,
        passthrough=params.passthrough,
    )
    values["extra_headers"] = step.extra_headers
    values["extra_body"] = step.extra_body
    match step:
        case UploadStep():
            values["file"] = (
                step.file.filename,
                step.file.content.encode(),
                step.file.content_type,
            )
            values["purpose"] = step.purpose
            expires = step.expires_after
            if expires is not None:
                values["expires_after"] = ExpiresAfter(anchor=expires.anchor, seconds=expires.seconds)
            # an explicit null reaches create_file, an absent provider keeps each function's default
            if "custom_llm_provider" in step.model_fields_set:
                values["custom_llm_provider"] = step.custom_llm_provider
        case DeleteStep():
            values["file_id"] = step.file_id
            values["model"] = step.model
            if step.custom_llm_provider is not None:
                values["custom_llm_provider"] = step.custom_llm_provider
    return values


# a record is known by the dump it answers; pydantic's BaseModel class object carries Any
@runtime_checkable
class Dumped(Protocol):
    def model_dump_json(self, *, exclude_none: bool) -> str: ...


# the betterlite records leave out null fields, so the LiteLLM record is dumped without them
def settle(result: WriteResult) -> JsonValue:
    match result:
        case Coroutine():
            return settle(asyncio.run(result))
        case Dumped():
            return JSON.validate_json(result.model_dump_json(exclude_none=True))


def rejection(error: openai.BadRequestError) -> Rejection:
    raised = Raised.model_validate(error)
    return Rejection(
        model=raised.model,
        llm_provider=raised.llm_provider,
        body=error.response.text,
        method=error.request.method,
        url=str(error.request.url),
    )


def outcome(call: WriteCall, values: WriteValues) -> StepOutcome:
    try:
        value = settle(call(**values))
    except BAD_REQUEST as error:
        return StepOutcome(failure=api_failure(error), bad_request=rejection(error))
    except openai.APIStatusError as error:
        return StepOutcome(failure=api_failure(error))
    except PROVIDER_ERROR as error:
        provider = ProviderFailure.model_validate(error)
        failure = Failure(
            cls=type(error).__name__, status=provider.status_code, message=provider.message
        )
        return StepOutcome(failure=failure)
    except ValueError as error:
        return StepOutcome(failure=Failure(cls=type(error).__name__, status=0, message=str(error)))
    return StepOutcome(value=value)


def run_step(step: Step, harness: str) -> StepOutcome:
    match step:
        case CreateKind(create_file=upload):
            return outcome(API.create_file, write_values(upload, harness))
        case ACreateKind(acreate_file=upload):
            return outcome(API.acreate_file, write_values(upload, harness))
        case DeleteKind(file_delete=delete):
            return outcome(API.file_delete, write_values(delete, harness))
        case ADeleteKind(afile_delete=delete):
            return outcome(API.afile_delete, write_values(delete, harness))
