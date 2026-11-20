import asyncio
import json
from collections.abc import Coroutine, Iterator, Mapping
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.files.transformation import BaseFileUploadStream
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.mistral.files.transformation import MistralFilesConfig
from litellm.types.llms.openai import CreateFileRequest, OpenAIFileObject
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

MultipartValue = dict[str, tuple[str | None, str | bytes, str | None]]
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class FileSpec(BaseModel):
    content: str | list[int]
    filename: str | None = None
    content_type: str | None = None


class RequestSpec(BaseModel):
    data: str | list[int] | dict[str, JsonValue]
    headers: dict[str, str] = Field(default_factory=dict)
    method: str | None = None
    url: str | None = None


class TwoStepSpec(BaseModel):
    initial_request: RequestSpec
    upload_request: RequestSpec
    upload_url_location: Literal["headers", "body"] | None = None
    upload_url_key: str | None = None


class PresignedSpec(BaseModel):
    method: str
    url: str
    data: str | list[int]
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

    mode: Literal["sync", "async"]
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


TwoStepValue = dict[str, RequestValue | str]
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


def content_value(content: str | list[int]) -> str | bytes:
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
        value["url"] = spec.url.replace("{{upstream}}", origin)
    return value


def transformed_value(plan: Plan, origin: str) -> Transformed:
    if plan.two_step is not None:
        spec = plan.two_step
        value: TwoStepValue = {
            "initial_request": request_value(spec.initial_request, origin),
            "upload_request": request_value(spec.upload_request, origin),
        }
        if spec.upload_url_location is not None:
            value["upload_url_location"] = spec.upload_url_location
        if spec.upload_url_key is not None:
            value["upload_url_key"] = spec.upload_url_key
        return value
    if plan.presigned is not None:
        return {
            "method": plan.presigned.method,
            "url": plan.presigned.url.replace("{{upstream}}", origin),
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
        super().__init__(model="", messages="", stream=False, call_type="create_file", start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="file-http-upload", function_id="file-http-upload")  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L556  # noqa: E501
        self.observed = None

    @override
    def pre_call(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        self.observed = {name: TypeAdapter(JsonValue).validate_json(json.dumps(value, default=list)) for name, value in kwargs.items()}  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L3643-L3658
        super().pre_call(*args, **kwargs)  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L1350


class FixtureConfig(MistralFilesConfig):
    def __init__(self, case: UploadCall, api_base: str) -> None:
        self.case = case
        self.api_base = api_base

    @property
    @override
    def file_upload_http_method(self) -> str:
        return self.case.upload_method.upper()

    @override
    def validate_environment(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L143
        self, headers: Mapping[str, str], *_args: JsonValue, **_kwargs: JsonValue) -> dict[str, str]:  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L143
        del headers
        return dict(self.case.headers)

    @override
    def get_complete_file_url(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L74
        self, api_base: str | None, *_args: JsonValue, **_kwargs: JsonValue) -> str | None:
        del api_base
        return self.api_base

    @override
    def transform_create_file_request(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L169
        self, model: str, create_file_data: CreateFileRequest,
        optional_params: dict[str, JsonValue],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L169
        litellm_params: dict[str, JsonValue],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L169
    ) -> Transformed:
        del model, create_file_data, optional_params, litellm_params  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1283
        return transformed_value(self.case.plan, self.api_base.removesuffix("/v1"))

    @override
    def transform_create_file_response(
        self, model: str | None, raw_response: httpx.Response, logging_obj: Logging,
        litellm_params: dict[str, JsonValue],  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/files/transformation.py#L187
    ) -> OpenAIFileObject:
        del model, logging_obj, litellm_params
        return OpenAIFileObject(
            id="file-oracle", object="file", bytes=len(raw_response.content),
            created_at=0, filename="oracle.txt", purpose="batch",
        )

    @override
    def get_error_class(
        self, error_message: str, status_code: int, headers: Mapping[str, str] | httpx.Headers
    ) -> BaseLLMException:
        return BaseLLMException(status_code, error_message, dict(headers))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L161


def diagnostics_view(case: UploadCall, logging_obj: FixtureLogging) -> JsonValue:
    observed = logging_obj.observed
    if case.mode == "sync" or observed is None:
        return None
    args = TypeAdapter(dict[str, JsonValue]).validate_python(observed["additional_args"])
    return {
        "input": observed.get("input"), "api_key": observed.get("api_key"), "api_base": args.get("api_base"),  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L3643-L3658
        "headers": args.get("headers"), "complete_input_dict": args.get("complete_input_dict"),
    }


class Outcome(BaseModel):
    result: JsonValue = None
    failure: Failure | None = None
    diagnostics: JsonValue = None


class UploadEnvelope(BaseModel):
    file_http_upload: dict[str, Outcome]


def run_upload(case: UploadCall, api_base: str) -> Outcome:
    logging_obj = FixtureLogging()
    filename = "oracle.txt" if case.file.filename is None else case.file.filename
    create_file_data: CreateFileRequest = {
        "file": (filename, content_value(case.file.content), case.file.content_type),  # type: ignore[typeddict-item]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L96
        "purpose": "batch",
    }
    try:
        work = BaseLLMHTTPHandler().create_file(
            create_file_data=create_file_data,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1283
            litellm_params=dict[str, JsonValue](case.litellm_params),
            provider_config=FixtureConfig(case, api_base),
            headers=dict[str, str](),
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
    except PROVIDER_ERROR as error:
        return Outcome(
            failure=Failure.model_validate(error),
            diagnostics=diagnostics_view(case, logging_obj),
        )
    return Outcome(
        result=TypeAdapter(JsonValue).validate_json(result.model_dump_json(exclude_unset=True)),
        diagnostics=diagnostics_view(case, logging_obj),
    )


class FileHttpUploadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_http_upload: UploadCall

    def serialize(self, api_base: str) -> str:
        outcome = run_upload(self.file_http_upload, api_base)
        envelope = UploadEnvelope(file_http_upload={"value": outcome})
        text = envelope.model_dump_json(by_alias=True)
        return text.replace(api_base.removesuffix("/v1"), "{{upstream}}")
