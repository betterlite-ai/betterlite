import io
import os
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypeVar, runtime_checkable
from unittest import mock

import httpx
import litellm
from litellm.exceptions import BadRequestError, UnsupportedParamsError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from litellm.llms.azure_ai.image_edit import get_azure_ai_image_edit_config
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

Call = TypeVar("Call")
OPENAI_ERROR: Final[type[Exception]] = OpenAIError
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError
BAD_REQUEST: Final[type[Exception]] = BadRequestError
CLOCK: Final = 1_700_000_000


class ImageField(TypedDict, total=False):
    kind: Literal["bytes", "buffered", "reader", "unsupported"]
    text: str
    position: int


class AuthStep(TypedDict):
    model: str
    api_key: NotRequired[str]
    global_api_key: NotRequired[str]
    global_api_base: NotRequired[str]
    api_base: NotRequired[str]
    environment: NotRequired[dict[str, str]]
    params: NotRequired[dict[str, JsonValue]]
    headers: NotRequired[dict[str, str]]


class UrlStep(TypedDict):
    model: str
    api_base: NotRequired[str]
    api_version: NotRequired[str]
    global_api_version: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class TransformStep(TypedDict):
    model: str
    prompt: NotRequired[str]
    image: NotRequired[ImageField]
    images: NotRequired[list[ImageField | None]]
    options: NotRequired[dict[str, JsonValue]]


class ReplyStep(TypedDict):
    body: str
    status: NotRequired[int]


class Input(TypedDict):
    models: NotRequired[list[str]]
    supported: NotRequired[list[str]]
    auths: NotRequired[list[AuthStep]]
    urls: NotRequired[list[UrlStep]]
    transforms: NotRequired[list[TransformStep]]
    replies: NotRequired[list[ReplyStep]]


@runtime_checkable
class Config(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        image_edit_optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def use_multipart_form_data(self) -> bool: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_image_edit_request(
        self,
        model: str,
        prompt: str | None,
        image: object,
        image_edit_optional_request_params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> tuple[dict[str, JsonValue], list[tuple[str, tuple[str, object, str]]]]: ...

    def transform_image_edit_response(
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> ImageResponse: ...


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: Config


class FailureRecord(BaseModel):
    status_code: int | None = None
    llm_provider: str | None = None


class LoggedDetails(BaseModel):
    input: str | None = None
    api_key: str | None = None
    original_response: str | None = None
    additional_args: dict[str, JsonValue] | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: LoggedDetails


class ProjectedImage(TypedDict, total=False):
    b64_json: str
    url: str
    revised_prompt: str


class UsageDetails(TypedDict):
    image_tokens: int
    text_tokens: int


class ProjectedUsage(TypedDict):
    input_tokens: int
    input_tokens_details: UsageDetails
    output_tokens: int
    total_tokens: int


OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
IMAGE_ADAPTER: TypeAdapter[ProjectedImage] = TypeAdapter(ProjectedImage)
USAGE_ADAPTER: TypeAdapter[ProjectedUsage] = TypeAdapter(ProjectedUsage)


def config_for(model: str) -> Config:
    values: Final = {"config": get_azure_ai_image_edit_config(model)}
    return ConfigBoundary.model_validate(values).config


@contextmanager
def environment(values: dict[str, str]) -> Generator[None]:
    with mock.patch.dict(os.environ, values):
        yield


@contextmanager
def global_settings(
    api_base: str | None, api_key: str | None, api_version: str | None
) -> Generator[None]:
    saved: Final = (
        litellm.api_base,
        litellm.api_key,
        litellm.api_version,
        litellm.enable_azure_ad_token_refresh,
    )
    litellm.api_base = api_base
    litellm.api_key = api_key
    litellm.api_version = api_version
    litellm.enable_azure_ad_token_refresh = False
    try:
        yield
    finally:
        (
            litellm.api_base,
            litellm.api_key,
            litellm.api_version,
            litellm.enable_azure_ad_token_refresh,
        ) = saved


@contextmanager
def frozen_clock() -> Generator[None]:
    with mock.patch.object(time, "time", return_value=float(CLOCK)):
        yield


def failure(kind: str, status: int | None, provider: str | None) -> JsonValue:
    return {"error": {"kind": kind, "status": status, "provider": provider}}


def outcomes(calls: list[Call], compute: Callable[[Call], JsonValue]) -> JsonValue:
    results: list[JsonValue] = []
    for call in calls:
        try:
            results.append({"value": compute(call)})
        except ValueError:
            results.append(failure("ValueError", None, None))
        except UNSUPPORTED as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(failure("UnsupportedParamsError", record.status_code, record.llm_provider))
        except BAD_REQUEST as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(failure("BadRequestError", record.status_code, record.llm_provider))
        except OPENAI_ERROR as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(failure("OpenAIError", record.status_code, None))
    return results


def model_value(model: str) -> JsonValue:
    try:
        config = config_for(model)
    except ValueError:
        return failure("ValueError", None, None)
    return {"class": type(config).__name__, "multipart": config.use_multipart_form_data()}


def supported_value(model: str) -> JsonValue:
    try:
        config = config_for(model)
    except ValueError:
        return failure("ValueError", None, None)
    allowlist: list[JsonValue] = []
    allowlist.extend(config.get_supported_openai_params(model))
    return allowlist


def auth_value(step: AuthStep) -> JsonValue:
    with environment(dict(step.get("environment", {}))):
        with global_settings(step.get("global_api_base"), step.get("global_api_key"), None):
            config = config_for(step["model"])
            try:
                built = config.validate_environment(
                    dict(step.get("headers", {})),
                    step["model"],
                    api_key=step.get("api_key"),
                    litellm_params=dict(step.get("params", {})),
                    api_base=step.get("api_base"),
                )
            except ValueError:
                return failure("ValueError", None, None)
            return {
                "class": type(config).__name__,
                "multipart": config.use_multipart_form_data(),
                "base": AzureFoundryModelInfo.get_api_base(step.get("api_base")),
                "headers": dict(built),
            }


def url_value(step: UrlStep) -> JsonValue:
    with environment(dict(step.get("environment", {}))):
        with global_settings(None, None, step.get("global_api_version")):
            config = config_for(step["model"])
            params: dict[str, JsonValue] = {}
            version = step.get("api_version")
            if version is not None:
                params["api_version"] = version
            return config.get_complete_url(step["model"], step.get("api_base"), params)


def image_value(field: ImageField) -> object:
    text: Final = field.get("text", "").encode()
    position: Final = field.get("position", 0)
    kind: Final = field.get("kind", "bytes")
    if kind == "buffered":
        buffered = io.BufferedReader(io.BytesIO(text))
        buffered.seek(position)
        return buffered
    if kind == "reader":
        reader = io.BytesIO(text)
        reader.seek(position)
        return reader
    if kind == "unsupported":
        return field.get("text", "")
    return text


def transform_value(step: TransformStep) -> JsonValue:
    config = config_for(step["model"])
    kind: Final = "mai" if "mai" in step["model"].lower() else "flux2"
    options: Final = dict(step.get("options", {}))
    mapped: Final = (
        config.map_openai_params(options, step["model"], False) if kind == "flux2" else options
    )
    images: Final = step.get("images")
    single: Final = step.get("image")
    if images is not None:
        image: object = [None if field is None else image_value(field) for field in images]
    elif single is not None:
        image = image_value(single)
    else:
        image = None
    data, files = config.transform_image_edit_request(
        step["model"],
        step.get("prompt"),
        image,
        mapped,
        GenericLiteLLMParams(),
        {},
    )
    projected: list[JsonValue] = []
    for name, payload in files:
        filename, _content, content_type = payload
        projected.append({"field": name, "filename": filename, "content_type": content_type})
    return {"kind": kind, "data": data, "files": projected}


def reply_value(step: ReplyStep) -> JsonValue:
    with frozen_clock():
        config = config_for("MAI-Image-2.5")
        logging_obj = Logging(
            model="fixture-model",
            messages=None,
            stream=False,
            call_type="image_generation",
            start_time=datetime(2026, 1, 1, tzinfo=UTC),
            litellm_call_id="call-1",
            function_id="fn-1",
        )
        try:
            response = config.transform_image_edit_response(
                "MAI-Image-2.5",
                httpx.Response(step.get("status", 200), content=step["body"].encode()),
                logging_obj,
            )
        except OPENAI_ERROR as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            return failure("OpenAIError", record.status_code, None)
        except ValueError:
            return failure("ValueError", None, None)
        details = RecordedLogging.model_validate(logging_obj).model_call_details
        logged: JsonValue = None
        if details.original_response is not None and details.additional_args is not None:
            logged = {
                "input": details.input,
                "api_key": details.api_key,
                "original_response": OBJECT.validate_json(details.original_response),
                "complete_input_dict": details.additional_args["complete_input_dict"],
            }
        data: list[JsonValue] = []
        for image in response.data or []:
            data.append(dict(IMAGE_ADAPTER.validate_json(image.model_dump_json(exclude_none=True))))
        usage = response.usage
        if usage is None:
            projected_usage: JsonValue = {
                "input_tokens": 0,
                "input_tokens_details": {"image_tokens": 0, "text_tokens": 0},
                "output_tokens": 0,
                "total_tokens": 0,
            }
        else:
            projected_usage = dict(USAGE_ADAPTER.validate_json(usage.model_dump_json()))
        return {
            "created": response.created,
            "data": data,
            "size": response.size,
            "usage": projected_usage,
            "logged": logged,
        }


def run(values: Input) -> JsonValue:
    answer: dict[str, JsonValue] = {}
    if "models" in values:
        answer["models"] = [model_value(model) for model in values["models"]]
    if "supported" in values:
        answer["supported"] = [supported_value(model) for model in values["supported"]]
    if "auths" in values:
        answer["auths"] = [auth_value(step) for step in values["auths"]]
    if "urls" in values:
        answer["urls"] = outcomes(values["urls"], url_value)
    if "transforms" in values:
        answer["transforms"] = outcomes(values["transforms"], transform_value)
    if "replies" in values:
        answer["replies"] = [reply_value(step) for step in values["replies"]]
    return answer
