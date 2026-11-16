import io
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Final, TypeVar

import litellm
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from .contracts import (
    BAD_REQUEST,
    OPENAI_ERROR,
    UNSUPPORTED,
    AuthStep,
    FailureRecord,
    Headers,
    ImageArgument,
    ImageField,
    ImageValue,
    JsonObject,
    TransformStep,
    UrlStep,
    config_for,
)

Call = TypeVar("Call")
CLOCK: Final = 1_700_000_000


@contextmanager
def environment(values: Headers) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


def apply_globals(api_base: str | None, api_key: str | None, api_version: str | None) -> None:
    litellm.api_base = api_base
    litellm.api_key = api_key
    litellm.api_version = api_version
    litellm.enable_azure_ad_token_refresh = False


def failure(kind: str, status: int | None, provider: str | None) -> JsonValue:
    detail: JsonObject = {"kind": kind, "status": status, "provider": provider}
    return {"error": detail}


def outcomes(calls: list[Call], compute: Callable[[Call], JsonValue]) -> JsonValue:
    results: list[JsonValue] = []
    for call in calls:
        try:
            results.append({"value": compute(call)})
        except ValueError:
            results.append(failure("ValueError", None, None))
        except (UNSUPPORTED, BAD_REQUEST) as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(failure(type(error).__name__, record.status_code, record.llm_provider))
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
    configured = step.get("environment")
    apply_globals(step.get("global_api_base"), step.get("global_api_key"), None)
    with environment({} if configured is None else configured):
        config = config_for(step["model"])
        supplied = step.get("headers")
        wired = step.get("params")
        try:
            built = config.validate_environment(
                {} if supplied is None else supplied,
                step["model"],
                api_key=step.get("api_key"),
                litellm_params=wired,
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
    configured = step.get("environment")
    apply_globals(None, None, step.get("global_api_version"))
    with environment({} if configured is None else configured):
        config = config_for(step["model"])
        params: JsonObject = {}
        version = step.get("api_version")
        if version is not None:
            params["api_version"] = version
        return config.get_complete_url(step["model"], step.get("api_base"), params)


def image_value(field: ImageField) -> ImageValue:
    text: Final = field["text"].encode()
    position = field.get("position")
    kind: Final = field["kind"]
    if kind == "buffered":
        buffered = io.BufferedReader(io.BytesIO(text))
        buffered.seek(0 if position is None else position)
        return buffered
    if kind == "reader":
        reader = io.BytesIO(text)
        reader.seek(0 if position is None else position)
        return reader
    if kind == "unsupported":
        return field["text"]
    return text


def transform_value(step: TransformStep) -> JsonValue:
    config = config_for(step["model"])
    kind: Final = "mai" if "mai" in step["model"].lower() else "flux2"
    configured = step.get("options")
    options: Final = {} if configured is None else configured
    mapped: Final = (
        config.map_openai_params(options, step["model"], drop_params=False)
        if kind == "flux2"
        else options
    )
    images = step.get("images")
    single = step.get("image")
    image: ImageArgument
    if images is not None:
        image = [None if field is None else image_value(field) for field in images]
    elif single is not None:
        image = image_value(single)
    else:
        image = None
    data, files = config.transform_image_edit_request(
        step["model"], step.get("prompt"), image, mapped, GenericLiteLLMParams(), {}
    )
    projected: list[JsonValue] = []
    for name, payload in files:
        filename, _content, content_type = payload
        projected.append({"field": name, "filename": filename, "content_type": content_type})
    return {"kind": kind, "data": data, "files": projected}
