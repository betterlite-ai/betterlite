import asyncio
import json
from collections.abc import Coroutine, Iterator, Mapping
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.files.transformation import BaseFileUploadStream
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.mistral.files.transformation import MistralFilesConfig
from litellm.types.llms.openai import CreateFileRequest, OpenAIFileObject
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

Mode = Literal["sync", "async"]
Content = str | list[int]
Location = Literal["headers", "body"]
MultipartValue = dict[str, tuple[str | None, str | bytes] | tuple[str | None, str | bytes, str]]

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HANDLER: Final = BaseLLMHTTPHandler()
PLACEHOLDER: Final = "{{upstream}}"
DEFAULT_FILENAME: Final = "oracle.txt"


class FileSpec(BaseModel):
    content: Content
    filename: str | None = None
    content_type: str | None = None


class RequestSpec(BaseModel):
    data: Content | dict[str, JsonValue]
    headers: dict[str, str] = Field(default_factory=dict)
    method: str | None = None
    url: str | None = None


class TwoStepSpec(BaseModel):
    initial_request: RequestSpec
    upload_request: RequestSpec
    upload_url_location: Location | None = None
    upload_url_key: str | None = None


class PresignedSpec(BaseModel):
    method: str
    url: str
    data: Content
    headers: dict[str, str] = Field(default_factory=dict)


class MediaSpec(BaseModel):
    content_type: str | None = None
    chunks: list[list[int]] = Field(default_factory=list)


class PartSpec(FileSpec):
    name: str


class MultipartSpec(BaseModel):
    parts: list[PartSpec]


class Plan(BaseModel):
    two_step: TwoStepSpec | None = None
    presigned: PresignedSpec | None = None
    media: MediaSpec | None = None
    multipart: MultipartSpec | None = None
    text: str | None = None
    raw: list[int] | None = None


class UploadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    file: FileSpec
    plan: Plan
    upload_method: Literal["post", "put"] = "post"
    headers: dict[str, str] = Field(default_factory=dict)
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)
    timeout_seconds: int | None = None


class RequestValue(TypedDict):
    headers: dict[str, str]
    data: JsonValue | bytes
    method: NotRequired[str]
    url: NotRequired[str]


TwoStepValue = dict[str, RequestValue | Location | str]
MediaValue = dict[str, dict[str, BaseFileUploadStream | str | None]]
Transformed = TwoStepValue | RequestValue | MediaValue | MultipartValue | str | bytes


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, JsonValue]


class ChunkStream(BaseFileUploadStream):
    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks

    @override
    def iter_bytes(self) -> Iterator[bytes]:
        return iter(self.chunks)


def content_value(content: Content) -> str | bytes:
    match content:
        case str():
            return content
        case list():
            return bytes(content)


def request_value(spec: RequestSpec, origin: str) -> RequestValue:
    data: JsonValue | bytes
    match spec.data:
        case list() as items:
            data = bytes(items)
        case str() | dict():
            data = spec.data
    value: RequestValue = {"headers": spec.headers, "data": data}
    if spec.method is not None:
        value["method"] = spec.method
    if spec.url is not None:
        value["url"] = spec.url.replace(PLACEHOLDER, origin)
    return value


def two_step_value(spec: TwoStepSpec, origin: str) -> TwoStepValue:
    value: TwoStepValue = {
        "initial_request": request_value(spec.initial_request, origin),
        "upload_request": request_value(spec.upload_request, origin),
    }
    if spec.upload_url_location is not None:
        value["upload_url_location"] = spec.upload_url_location
    if spec.upload_url_key is not None:
        value["upload_url_key"] = spec.upload_url_key
    return value


def transformed_value(plan: Plan, origin: str) -> Transformed:
    if plan.two_step is not None:
        return two_step_value(plan.two_step, origin)
    if plan.presigned is not None:
        return {
            "method": plan.presigned.method,
            "url": plan.presigned.url.replace(PLACEHOLDER, origin),
            "headers": plan.presigned.headers,
            "data": content_value(plan.presigned.data),
        }
    if plan.media is not None:
        stream = ChunkStream([bytes(chunk) for chunk in plan.media.chunks])
        body: dict[str, BaseFileUploadStream | str | None] = {"body_stream": stream}
        body["content_type"] = plan.media.content_type
        return {"streaming_media_upload": body}
    if plan.multipart is not None:
        return {
            part.name: (part.filename, content_value(part.content), part.content_type)
            for part in plan.multipart.parts
        }
    if plan.text is not None:
        return plan.text
    if plan.raw is not None:
        return bytes(plan.raw)
    raise TypeError("the case names no plan")


class FixtureLogging(Logging):
    observed: dict[str, JsonValue] | None

    def __init__(self) -> None:
        super().__init__("", "", False, "create_file", datetime(2026, 1, 1, tzinfo=UTC), "file-http-upload", "file-http-upload")  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L556  # noqa: E501
        self.observed = None

    @override
    def pre_call(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        self.observed = {name: JSON.validate_json(json.dumps(value, default=list)) for name, value in kwargs.items()}
        super().pre_call(*args, **kwargs)  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L1350  # noqa: E501


class FixtureConfig(MistralFilesConfig):
    def __init__(self, case: UploadCall, api_base: str) -> None:
        super().__init__()
        self.case = case
        self.api_base = api_base

    @property
    @override
    def file_upload_http_method(self) -> str:
        return self.case.upload_method.upper()

    @override
    def validate_environment(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L143  # noqa: E501
        self, headers: Mapping[str, str], *_args: JsonValue, **_kwargs: JsonValue) -> dict[str, str]:
        del headers
        return dict(self.case.headers)

    @override
    def get_complete_file_url(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L74  # noqa: E501
        self, api_base: str | None, *_args: JsonValue, **_kwargs: JsonValue) -> str | None:
        del api_base
        return self.api_base

    @override
    def transform_create_file_request(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L169  # noqa: E501
        self, model: str, create_file_data: CreateFileRequest,
        optional_params: dict[str, JsonValue],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L169  # noqa: E501
        litellm_params: dict[str, JsonValue],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L169  # noqa: E501
    ) -> TwoStepValue | RequestValue | MediaValue | MultipartValue | str | bytes:
        del model, create_file_data, optional_params, litellm_params  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1283  # noqa: E501
        return transformed_value(self.case.plan, self.api_base.removesuffix("/v1"))

    @override
    def transform_create_file_response(
        self, model: str | None, raw_response: httpx.Response, logging_obj: Logging,
        litellm_params: dict[str, JsonValue],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L187  # noqa: E501
    ) -> OpenAIFileObject:
        del model, logging_obj, litellm_params
        return OpenAIFileObject(
            id="file-oracle", object="file", bytes=len(raw_response.content),
            created_at=0, filename=DEFAULT_FILENAME, purpose="batch",
        )

    @override
    def get_error_class(
        self, error_message: str, status_code: int, headers: Mapping[str, str] | httpx.Headers
    ) -> BaseLLMException:
        return BaseLLMException(status_code, error_message, dict(headers))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L161  # noqa: E501


def diagnostics_view(case: UploadCall, logging_obj: FixtureLogging) -> JsonValue:
    observed = logging_obj.observed
    if case.mode == "sync" or observed is None:
        return None
    args = TypeAdapter(dict[str, JsonValue]).validate_python(observed["additional_args"])
    return {
        "input": observed.get("input"), "api_key": observed.get("api_key"), "api_base": args.get("api_base"),
        "headers": args.get("headers"), "complete_input_dict": args.get("complete_input_dict"),
    }


def run_upload(case: UploadCall, api_base: str) -> JsonValue:
    logging_obj = FixtureLogging()
    filename = DEFAULT_FILENAME if case.file.filename is None else case.file.filename
    create_file_data: CreateFileRequest = {
        "file": (filename, content_value(case.file.content), case.file.content_type),
        "purpose": "batch",
    }
    try:
        work = HANDLER.create_file(
            create_file_data=create_file_data,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1283  # noqa: E501
            litellm_params=dict(case.litellm_params),
            provider_config=FixtureConfig(case, api_base),
            headers={},
            api_base=api_base,
            api_key=None,
            logging_obj=logging_obj,
            _is_async=case.mode == "async",
            timeout=None if case.timeout_seconds is None else float(case.timeout_seconds),
        )
        result: OpenAIFileObject
        match work:
            case OpenAIFileObject():
                result = work
            case Coroutine():
                result = asyncio.run(work)
    except BaseLLMException as error:
        failed = Failure.model_validate(error)
        return {
            "result": None, "failure": JSON.validate_json(failed.model_dump_json(by_alias=True)),
            "diagnostics": diagnostics_view(case, logging_obj),
        }
    return {
        "result": JSON.validate_json(result.model_dump_json(exclude_unset=True)), "failure": None,
        "diagnostics": diagnostics_view(case, logging_obj),
    }


class FileHttpUploadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_http_upload: UploadCall

    def serialize(self, api_base: str) -> str:
        outcome = run_upload(self.file_http_upload, api_base)
        text = json.dumps({"file_http_upload": {"value": outcome}})
        return text.replace(api_base.removesuffix("/v1"), PLACEHOLDER)
