import asyncio
from typing import Final, Literal, TypeAlias
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
from litellm.types.utils import FileTypes, ImageResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

Mode = Literal["sync", "async"]
CompletedFiles: TypeAlias = list[tuple[str, tuple[str | None, bytes, str | None]]]
CompleteInput: TypeAlias = dict[str, JsonValue] | CompletedFiles

JSON: Final = TypeAdapter(JsonValue)
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
DATA: Final = TypeAdapter(dict[str, JsonValue])
FILES: Final = TypeAdapter(CompletedFiles)
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


class PreCall(BaseModel):
    input: JsonValue = None
    complete_input_dict: CompleteInput
    api_base: str
    headers: dict[str, JsonValue]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str]


class LoggedCall(BaseModel):
    input: JsonValue = None
    complete_input_dict: JsonValue = None
    header_names: list[str]
    api_base_path: str


class Outcome(BaseModel):
    result: JsonValue = None
    provider_headers: dict[str, str] | None = None
    failure: Failure | None = None
    logged: LoggedCall | None = None


class EditEnvelope(BaseModel):
    image_edit_http: dict[str, Outcome]


class FixtureConfig(BaseImageEditConfig):
    def __init__(self, *, multipart: bool) -> None:
        self.multipart = multipart

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return []

    @override
    def map_openai_params(
        self,
        image_edit_optional_params: ImageEditOptionalRequestParams,
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]:
        return {}

    @override
    def validate_environment(
        self,
        headers: dict[str, JsonValue],
        model: str,
        api_key: str | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
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
        litellm_params: dict[str, JsonValue],
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
        # ast-grep-ignore: no-long-names
        image_edit_optional_request_params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> tuple[dict[str, str], list[tuple[str, FileTypes]]]:
        data: dict[str, str] = {}
        if prompt is not None:
            data["prompt"] = prompt
        files: list[tuple[str, FileTypes]] = []
        if image is not None:
            files.append(("image", image))
        return data, files

    @override
    # ast-grep-ignore: no-long-names
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
        return IMAGE_RESPONSE.validate_python(JSON.validate_json(raw_response.content))


HANDLER: Final = BaseLLMHTTPHandler()


class RecordingLogging(Logging):
    def __init__(self) -> None:
        self.logged: PreCall | None = None

    @override
    def pre_call(self, *args: JsonValue, **values: JsonValue) -> None:
        record = PreCall.model_validate(values["additional_args"])
        record.input = args[0] if args else values["input"]
        self.logged = record


def text_or_none(content: bytes) -> str | None:
    try:
        return content.decode("utf-8")
    except UnicodeDecodeError:
        return None


def files_mapping(files: CompletedFiles) -> dict[str, JsonValue]:
    mapping: dict[str, JsonValue] = {}
    for name, part in files:
        mapping[name] = {
            "filename": part[0],
            "content": text_or_none(part[1]),
            "content_type": part[2],
        }
    return mapping


def complete_value(value: CompleteInput, mode: Mode) -> JsonValue:
    if mode == "sync":
        return JSON.validate_python(files_mapping(FILES.validate_python(value)))
    return JSON.validate_python(DATA.validate_python(value))


def logged_call(record: PreCall | None, mode: Mode) -> LoggedCall | None:
    if record is None:
        return None
    return LoggedCall(
        input=record.input,
        complete_input_dict=complete_value(record.complete_input_dict, mode),
        header_names=sorted(record.headers),
        api_base_path=urlsplit(record.api_base).path,
    )


def run_edit(call: EditCall, api_base: str) -> Outcome:
    logging_obj = RecordingLogging()
    config = FixtureConfig(multipart=call.multipart)
    litellm_params = GenericLiteLLMParams(api_base=api_base, api_key=call.api_key)
    image: FileTypes = (call.image.filename, call.image.content.encode(), call.image.content_type)
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
            answer = IMAGE_RESPONSE.validate_python(
                asyncio.run(
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
                ),
            )
    except PROVIDER_ERROR as error:
        return Outcome(
            failure=Failure.model_validate(error),
            logged=logged_call(logging_obj.logged, call.mode),
        )
    stored = get_provider_response_headers_from_hidden_params(answer)
    return Outcome(
        result=JSON.validate_json(answer.model_dump_json(exclude_none=True, exclude={"usage"})),
        provider_headers=None if stored is None else dict(stored),
        logged=logged_call(logging_obj.logged, call.mode),
    )


class ImageEditHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_edit_http: EditCall

    def serialize(self, api_base: str) -> str:
        outcome = run_edit(self.image_edit_http, api_base)
        return EditEnvelope(image_edit_http={"value": outcome}).model_dump_json(by_alias=True)
