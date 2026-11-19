import asyncio
from collections.abc import Coroutine, Iterator, Mapping
from datetime import UTC, datetime
from typing import Final, Literal

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.files.transformation import BaseFileUploadStream
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.mistral.files.transformation import MistralFilesConfig
from litellm.types.llms.openai import CreateFileRequest, OpenAIFileObject
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import NotRequired, TypedDict, override

Mode = Literal["sync", "async"]
UploadMethod = Literal["post", "put"]
Content = str | list[int]
UploadUrlLocation = Literal["headers", "body"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
HANDLER: Final = BaseLLMHTTPHandler()
PLACEHOLDER: Final = "{{upstream}}"
DEFAULT_FILENAME: Final = "oracle.txt"

PartValue = tuple[str | None, bytes, str | None] | tuple[None, bytes]
MultipartValue = dict[str, PartValue]


class InitialRequestValue(TypedDict):
    headers: dict[str, str]
    data: dict[str, JsonValue]
    method: NotRequired[str]
    url: NotRequired[str]


class UploadRequestValue(TypedDict):
    headers: dict[str, str]
    data: str | bytes
    method: NotRequired[str]
    url: NotRequired[str]


class TwoStepValue(TypedDict):
    initial_request: InitialRequestValue
    upload_request: UploadRequestValue
    upload_url_location: NotRequired[UploadUrlLocation]
    upload_url_key: NotRequired[str]


class PresignedValue(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    data: str | bytes


class StreamingMediaValue(TypedDict):
    body_stream: BaseFileUploadStream
    content_type: str | None


class MediaValue(TypedDict):
    streaming_media_upload: StreamingMediaValue


TransformedRequest = TwoStepValue | PresignedValue | MediaValue | MultipartValue | str | bytes


class FileSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filename: str | None = None
    content_type: str | None = None
    content: Content


class InitialRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    method: str | None = None
    url: str | None = None
    headers: dict[str, str] = Field(default_factory=dict)
    data: dict[str, JsonValue]


class UploadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    method: str | None = None
    url: str | None = None
    headers: dict[str, str] = Field(default_factory=dict)
    data: Content


class TwoStepSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    initial_request: InitialRequest
    upload_request: UploadRequest
    upload_url_location: UploadUrlLocation | None = None
    upload_url_key: str | None = None


class TwoStepPlan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    two_step: TwoStepSpec


class PresignedSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    method: str
    url: str
    headers: dict[str, str] = Field(default_factory=dict)
    data: Content


class PresignedPlan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    presigned: PresignedSpec


class MediaSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    content_type: str | None = None
    chunks: list[list[int]] = Field(default_factory=list)


class MediaPlan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    media: MediaSpec


class PartSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    filename: str | None = None
    content_type: str | None = None
    content: Content


class MultipartSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    parts: list[PartSpec]


class MultipartPlan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    multipart: MultipartSpec


class TextPlan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    text: str


class RawPlan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    raw: list[int]


Plan = TwoStepPlan | PresignedPlan | MediaPlan | MultipartPlan | TextPlan | RawPlan


class UploadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    upload_method: UploadMethod = "post"
    headers: dict[str, str] = Field(default_factory=dict)
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)
    file: FileSpec
    timeout_seconds: int | None = None
    plan: Plan


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, JsonValue]


class Diagnostics(BaseModel):
    model_config = ConfigDict(extra="forbid")

    input: JsonValue
    api_key: JsonValue
    api_base: JsonValue
    headers: JsonValue
    complete_input_dict: JsonValue


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None
    diagnostics: Diagnostics | None = None


class UploadValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Outcome


class UploadEnvelope(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_http_upload: UploadValue


class AdditionalArgs(BaseModel):
    model_config = ConfigDict(extra="ignore")

    api_base: JsonValue = None
    headers: JsonValue = None
    complete_input_dict: JsonValue = None


class PreCallSnapshot(BaseModel):
    model_config = ConfigDict(extra="ignore")

    input: JsonValue
    api_key: JsonValue
    additional_args: AdditionalArgs = Field(default_factory=AdditionalArgs)


class ChunkStream(BaseFileUploadStream):
    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks

    @override
    def iter_bytes(self) -> Iterator[bytes]:
        return iter(self.chunks)


class FixtureLogging(Logging):
    observed: dict[str, JsonValue] | None

    def __init__(self) -> None:
        super().__init__(
            model="",
            messages="",
            stream=False,
            call_type="create_file",
            start_time=datetime(2026, 1, 1, tzinfo=UTC),
            litellm_call_id="file-http-upload",
            function_id="file-http-upload",
        )
        self.observed = None

    @override
    def pre_call(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        self.observed = kwargs
        super().pre_call(*args, **kwargs)


class FixtureConfig(MistralFilesConfig):
    case: UploadCall
    api_base: str

    def __init__(self, case: UploadCall, api_base: str) -> None:
        super().__init__()
        self.case = case
        self.api_base = api_base

    @property
    def origin(self) -> str:
        return self.api_base.removesuffix("/v1")

    @property
    @override
    def file_upload_http_method(self) -> str:
        return self.case.upload_method.upper()

    @override
    def validate_environment(
        self, headers: Mapping[str, str], *_args: JsonValue, **_kwargs: JsonValue
    ) -> dict[str, str]:
        del headers
        return dict(self.case.headers)

    @override
    def get_complete_file_url(
        self, api_base: str | None, *_args: JsonValue, **_kwargs: JsonValue
    ) -> str | None:
        del api_base
        return self.api_base

    @override
    def transform_create_file_request(
        self,
        model: str,
        create_file_data: CreateFileRequest,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> TransformedRequest:
        del model, create_file_data, optional_params, litellm_params
        return transformed_request(self.case.plan, self.origin)

    @override
    def transform_create_file_response(
        self,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: dict[str, JsonValue],
    ) -> OpenAIFileObject:
        del model, logging_obj, litellm_params
        return OpenAIFileObject(
            id="file-oracle",
            object="file",
            bytes=len(raw_response.content),
            created_at=raw_response.status_code,
            filename=self.case.file.filename or DEFAULT_FILENAME,
            purpose="batch",
        )

    @override
    def get_error_class(
        self,
        error_message: str,
        status_code: int,
        headers: Mapping[str, str] | httpx.Headers,
    ) -> BaseLLMException:
        return BaseLLMException(
            status_code=status_code, message=error_message, headers=dict(headers)
        )


def content_bytes(content: Content) -> bytes:
    value: bytes
    match content:
        case str():
            value = content.encode()
        case list():
            value = bytes(content)
    return value


def content_value(content: Content) -> str | bytes:
    value: str | bytes
    match content:
        case str():
            value = content
        case list():
            value = bytes(content)
    return value


def initial_request_value(spec: InitialRequest, origin: str) -> InitialRequestValue:
    value: InitialRequestValue = {"headers": dict(spec.headers), "data": dict(spec.data)}
    if spec.method is not None:
        value["method"] = spec.method
    if spec.url is not None:
        value["url"] = spec.url.replace(PLACEHOLDER, origin)
    return value


def upload_request_value(spec: UploadRequest, origin: str) -> UploadRequestValue:
    data = content_value(spec.data)
    value: UploadRequestValue = {"headers": dict(spec.headers), "data": data}
    if spec.method is not None:
        value["method"] = spec.method
    if spec.url is not None:
        value["url"] = spec.url.replace(PLACEHOLDER, origin)
    return value


def two_step_request(spec: TwoStepSpec, origin: str) -> TwoStepValue:
    value: TwoStepValue = {
        "initial_request": initial_request_value(spec.initial_request, origin),
        "upload_request": upload_request_value(spec.upload_request, origin),
    }
    if spec.upload_url_location is not None:
        value["upload_url_location"] = spec.upload_url_location
    if spec.upload_url_key is not None:
        value["upload_url_key"] = spec.upload_url_key
    return value


def presigned_request(spec: PresignedSpec, origin: str) -> PresignedValue:
    return {
        "method": spec.method,
        "url": spec.url.replace(PLACEHOLDER, origin),
        "headers": dict(spec.headers),
        "data": content_value(spec.data),
    }


def media_request(spec: MediaSpec) -> MediaValue:
    stream = ChunkStream([bytes(chunk) for chunk in spec.chunks])
    return {"streaming_media_upload": {"body_stream": stream, "content_type": spec.content_type}}


def part_tuple(part: PartSpec) -> PartValue:
    data = content_bytes(part.content)
    if part.filename is None and part.content_type is None:
        return (None, data)
    return (part.filename, data, part.content_type)


def multipart_request(spec: MultipartSpec) -> MultipartValue:
    form: MultipartValue = {}
    for part in spec.parts:
        form[part.name] = part_tuple(part)
    return form


def transformed_request(plan: Plan, origin: str) -> TransformedRequest:
    value: TransformedRequest
    match plan:
        case TwoStepPlan():
            value = two_step_request(plan.two_step, origin)
        case PresignedPlan():
            value = presigned_request(plan.presigned, origin)
        case MediaPlan():
            value = media_request(plan.media)
        case MultipartPlan():
            value = multipart_request(plan.multipart)
        case TextPlan():
            value = plan.text
        case RawPlan():
            value = bytes(plan.raw)
    return value


def diagnostics_view(mode: Mode, logging_obj: FixtureLogging) -> Diagnostics | None:
    if mode == "sync":
        return None
    observed = logging_obj.observed
    if observed is None:
        return None
    snapshot = PreCallSnapshot.model_validate(observed)
    arguments = snapshot.additional_args
    return Diagnostics(
        input=snapshot.input,
        api_key=snapshot.api_key,
        api_base=arguments.api_base,
        headers=arguments.headers,
        complete_input_dict=arguments.complete_input_dict,
    )


def run_upload(case: UploadCall, api_base: str) -> Outcome:
    config = FixtureConfig(case, api_base)
    logging_obj = FixtureLogging()
    create_file_data: CreateFileRequest = {
        "file": (
            case.file.filename or DEFAULT_FILENAME,
            content_bytes(case.file.content),
            case.file.content_type,
        ),
        "purpose": "batch",
    }
    params: dict[str, JsonValue] = dict(case.litellm_params)
    headers: dict[str, JsonValue] = {}
    timeout = None if case.timeout_seconds is None else float(case.timeout_seconds)
    try:
        work = HANDLER.create_file(
            create_file_data=create_file_data,
            litellm_params=params,
            provider_config=config,
            headers=headers,
            api_base=api_base,
            api_key=None,
            logging_obj=logging_obj,
            _is_async=case.mode == "async",
            timeout=timeout,
        )
        result: OpenAIFileObject
        match work:
            case OpenAIFileObject():
                result = work
            case Coroutine():
                result = asyncio.run(work)
    except PROVIDER_ERROR as error:
        return Outcome(
            failure=Failure.model_validate(error),
            diagnostics=diagnostics_view(case.mode, logging_obj),
        )
    return Outcome(
        result=JSON.validate_python(result.model_dump(exclude_unset=True)),
        diagnostics=diagnostics_view(case.mode, logging_obj),
    )


class FileHttpUploadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_http_upload: UploadCall

    def serialize(self, api_base: str) -> str:
        outcome = run_upload(self.file_http_upload, api_base)
        envelope = UploadEnvelope(file_http_upload=UploadValue(value=outcome))
        return envelope.model_dump_json().replace(api_base.removesuffix("/v1"), PLACEHOLDER)
