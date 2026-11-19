import asyncio
from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Any, Final, Literal, override
from urllib.parse import urlsplit

import httpx
from litellm.litellm_core_utils.core_helpers import (
    get_provider_response_headers_from_hidden_params,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.images.main import ImageEditOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import FileTypes
from litellm.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode = Literal["sync", "async"]
FilePart = tuple[str | None, bytes, str | None]
FileEntry = tuple[str, Any]
CompletedFiles = list[tuple[str, FilePart]]

JSON: Final = TypeAdapter(JsonValue)
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
DETAILS: Final = TypeAdapter(dict[str, object])
COMPLETE: Final = TypeAdapter(dict[str, JsonValue] | CompletedFiles)
IMAGE_RESPONSE: Final = TypeAdapter(ImageResponse)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class ImageInput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filename: str
    content: str
    content_type: str | None = None


class EditCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    model: str
    prompt: str | None = None
    image: ImageInput
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    api_key: str | None = None
    ssl_verify: bool | None = None
    extra_headers: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    fake_stream: bool = False
    litellm_metadata: dict[str, JsonValue] | None = None
    multipart: bool = True
    timeout_seconds: int


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str]


class LoggedCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    input: JsonValue = None
    complete_input_dict: JsonValue = None
    header_names: list[str]
    api_base_path: str


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    provider_headers: dict[str, str] | None = None
    failure: Failure | None = None
    logged: LoggedCall | None = None


class EditValue(BaseModel):
    value: Outcome


class EditEnvelope(BaseModel):
    image_edit_http: EditValue


class FixtureConfig(BaseImageEditConfig):
    def __init__(self, *, multipart: bool) -> None:
        super().__init__()
        self.multipart: bool = multipart

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return []

    @override
    def map_openai_params(
        self,
        image_edit_optional_params: ImageEditOptionalRequestParams,
        model: str,
        drop_params: bool,
    ) -> dict[str, object]:
        return {}

    @override
    def validate_environment(
        self,
        headers: dict[str, object],
        model: str,
        api_key: str | None = None,
        litellm_params: dict[str, object] | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]:
        validated: dict[str, str] = {"x": "validated"}
        if api_key is not None:
            validated["authorization"] = f"Bearer {api_key}"
        return validated

    @override
    def get_complete_url(
        self,
        model: str,
        api_base: str | None,
        litellm_params: dict[str, object],
    ) -> str:
        if api_base is None:
            raise ValueError("api_base is required")
        return f"{api_base}/images/edits"

    @override
    def transform_image_edit_request(
        self,
        model: str,
        prompt: str | None,
        image: FileTypes | None,
        image_edit_optional_request_params: dict[str, object],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> tuple[dict[str, str], list[FileEntry]]:
        data: dict[str, str] = {}
        if prompt is not None:
            data["prompt"] = prompt
        files: list[FileEntry] = []
        if image is not None:
            files.append(("image", image))
        return data, files

    @override
    async def async_transform_image_edit_request(
        self,
        model: str,
        prompt: str | None,
        image: FileTypes | None,
        image_edit_optional_request_params: Mapping[str, object],
        litellm_params: GenericLiteLLMParams,
        headers: Mapping[str, str],
    ) -> tuple[dict[str, str], list[FileEntry]]:
        return self.transform_image_edit_request(
            model=model,
            prompt=prompt,
            image=image,
            image_edit_optional_request_params=dict(image_edit_optional_request_params),
            litellm_params=litellm_params,
            headers=dict(headers),
        )

    @override
    def finalize_image_edit_request_data(
        self,
        data: dict[str, str],
        resolved_request_url: str,
    ) -> dict[str, str]:
        data["revision"] = "final"
        return data

    @override
    def use_multipart_form_data(self) -> bool:
        return self.multipart

    @override
    def transform_image_edit_response(
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
    ) -> ImageResponse:
        return IMAGE_RESPONSE.validate_python(JSON.validate_python(raw_response.json()))


HANDLER: Final = BaseLLMHTTPHandler()


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="image_edit",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="image-edit-http",
        function_id="image-edit-http",
    )


def text_or_none(content: bytes) -> str | None:
    try:
        return content.decode("utf-8")
    except UnicodeDecodeError:
        return None


def file_value(part: FilePart) -> dict[str, object]:
    filename, content, content_type = part
    return {
        "filename": filename,
        "content": text_or_none(content),
        "content_type": content_type,
    }


def complete_value(value: object) -> JsonValue:
    completed = COMPLETE.validate_python(value)
    if isinstance(completed, dict):
        return JSON.validate_python(completed)
    return JSON.validate_python(
        {name: file_value(part) for name, part in completed},
    )


def header_names(value: object) -> list[str]:
    headers = OBJECTS.validate_python(value)
    if headers is None:
        return []
    return sorted(headers.keys())


def api_base_path(value: object) -> str:
    if isinstance(value, str):
        return urlsplit(value).path
    return ""


def logged_call(logging_obj: Logging) -> LoggedCall | None:
    details = DETAILS.validate_python(logging_obj.model_call_details)
    args = OBJECTS.validate_python(details.get("additional_args"))
    if args is None:
        return None
    return LoggedCall(
        input=JSON.validate_python(details.get("input")),
        complete_input_dict=complete_value(args.get("complete_input_dict")),
        header_names=header_names(args.get("headers")),
        api_base_path=api_base_path(args.get("api_base")),
    )


def result_value(response: ImageResponse) -> JsonValue:
    return JSON.validate_python(
        response.model_dump(mode="json", exclude_none=True, exclude={"usage"}),
    )


def provider_headers(response: ImageResponse) -> dict[str, str] | None:
    headers = get_provider_response_headers_from_hidden_params(response)
    if headers is None:
        return None
    return dict(headers)


def succeeded(response: ImageResponse, logging_obj: Logging) -> Outcome:
    return Outcome(
        result=result_value(response),
        provider_headers=provider_headers(response),
        logged=logged_call(logging_obj),
    )


def run_edit(call: EditCall, api_base: str) -> Outcome:
    logging_obj = logging_fixture()
    config = FixtureConfig(multipart=call.multipart)
    # ssl_verify is an extra field the handler reads through GenericLiteLLMParams' extra="allow" config
    litellm_params = GenericLiteLLMParams.model_validate(
        {"api_base": api_base, "api_key": call.api_key, "ssl_verify": call.ssl_verify},
    )
    image: FileTypes = (
        call.image.filename,
        call.image.content.encode(),
        call.image.content_type,
    )
    extra_headers = OBJECTS.validate_python(call.extra_headers)
    extra_body = OBJECTS.validate_python(call.extra_body)
    litellm_metadata = OBJECTS.validate_python(call.litellm_metadata)
    optional = dict(call.optional_params)
    answer: ImageResponse
    try:
        if call.mode == "sync":
            answer = IMAGE_RESPONSE.validate_python(
                HANDLER.image_edit_handler(
                    model=call.model,
                    image=image,
                    prompt=call.prompt,
                    image_edit_provider_config=config,
                    image_edit_optional_request_params=optional,
                    custom_llm_provider="openai",
                    litellm_params=litellm_params,
                    logging_obj=logging_obj,
                    timeout=float(call.timeout_seconds),
                    extra_headers=extra_headers,
                    extra_body=extra_body,
                    fake_stream=call.fake_stream,
                    litellm_metadata=litellm_metadata,
                ),
            )
        else:
            answer = asyncio.run(
                HANDLER.async_image_edit_handler(
                    model=call.model,
                    image=image,
                    prompt=call.prompt,
                    image_edit_provider_config=config,
                    image_edit_optional_request_params=optional,
                    custom_llm_provider="openai",
                    litellm_params=litellm_params,
                    logging_obj=logging_obj,
                    timeout=float(call.timeout_seconds),
                    extra_headers=extra_headers,
                    extra_body=extra_body,
                    fake_stream=call.fake_stream,
                    litellm_metadata=litellm_metadata,
                ),
            )
    except PROVIDER_ERROR as error:
        return Outcome(
            failure=Failure.model_validate(error),
            logged=logged_call(logging_obj),
        )
    return succeeded(answer, logging_obj)


class ImageEditHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_edit_http: EditCall

    def serialize(self, api_base: str) -> str:
        outcome = run_edit(self.image_edit_http, api_base)
        value = EditValue(value=outcome)
        return EditEnvelope(image_edit_http=value).model_dump_json(by_alias=True)
