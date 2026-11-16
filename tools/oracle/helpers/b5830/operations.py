import base64
import os
from dataclasses import dataclass
from io import BytesIO
from typing import Final

import litellm
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue

from .configs import EDIT, GENERATION
from .inputs import (
    EditRequestCase,
    GenerationParamsCase,
    GenerationRequestCase,
    ImageCase,
    Input,
    JSON,
    Object,
    ReplyCase,
)
from .replies import edit_fields, generation_fields, raw_response

MANAGED_ENV: Final = ("OPENROUTER_API_KEY", "OPENROUTER_API_BASE")


@dataclass
class ImageSource:
    value: bytes | BytesIO | None
    reader: BytesIO | None

    def position(self) -> int | None:
        if self.reader is None:
            return None
        return self.reader.tell()


def activate(environment: dict[str, str], global_api_key: str | None) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    os.environ.update(environment)
    litellm.api_key = global_api_key


def image_source(image: ImageCase | None) -> ImageSource:
    if image is None:
        return ImageSource(None, None)
    data = base64.b64decode(image["data"])
    if image["kind"] == "bytes":
        return ImageSource(data, None)
    stream = BytesIO(data)
    stream.seek(image["position"])
    return ImageSource(stream, stream)


def generation_params(cases: list[GenerationParamsCase]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        optional = dict(case["optional_params"])
        mapped = GENERATION.map_openai_params(
            dict(case["non_default_params"]), optional, case["model"], drop_params=case["drop_params"]
        )
        supported = GENERATION.get_supported_openai_params(case["model"])
        answers.append({"supported": JSON.validate_python(supported), "params": mapped})
    return JSON.validate_python(answers)


def generation_request(cases: list[GenerationRequestCase]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        activate(case["environment"], case["global_api_key"])
        url = GENERATION.get_complete_url(case["api_base"], case["api_key"], case["model"], {}, {})
        headers = GENERATION.validate_environment(
            dict(case["headers"]), case["model"], [], {}, {}, case["api_key"], case["api_base"]
        )
        request = GENERATION.transform_image_generation_request(
            case["model"], case["prompt"], dict(case["optional_params"]), {}, headers
        )
        answers.append({"url": url, "headers": JSON.validate_python(headers), "request": request})
    return JSON.validate_python(answers)


def generation_reply(cases: list[ReplyCase]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        response = ImageResponse(created=case["created"])
        finished = GENERATION.transform_image_generation_response(
            case["model"], raw_response(case), response, None, {}, {}, {}, None
        )
        answers.append(generation_fields(finished))
    return JSON.validate_python(answers)


def edit_params(cases: list[Object]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        supported = EDIT.get_supported_openai_params("")
        mapped = EDIT.map_openai_params(dict(case), "", drop_params=False)
        answers.append({"supported": JSON.validate_python(supported), "params": mapped})
    return JSON.validate_python(answers)


def edit_request(cases: list[EditRequestCase]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        activate(case["environment"], case["global_api_key"])
        url = EDIT.get_complete_url(case["model"], case["api_base"], {})
        headers = EDIT.validate_environment(
            dict(case["headers"]), case["model"], case["api_key"], None, case["api_base"]
        )
        source = image_source(case["image"])
        request, files = EDIT.transform_image_edit_request(
            case["model"], case["prompt"], source.value, dict(case["optional_params"]),
            GenericLiteLLMParams(), headers,
        )
        answers.append(
            {"url": url, "headers": JSON.validate_python(headers), "request": JSON.validate_python(request),
             "files": JSON.validate_python(files), "position": source.position()}
        )
    return JSON.validate_python(answers)


def edit_reply(cases: list[ReplyCase]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        finished = EDIT.transform_image_edit_response(case["model"], raw_response(case), None)
        answers.append(edit_fields(finished))
    return JSON.validate_python(answers)


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    match input_value["op"]:
        case "generation_params":
            result = generation_params(input_value["cases"])
        case "generation_request":
            result = generation_request(input_value["cases"])
        case "generation_reply":
            result = generation_reply(input_value["cases"])
        case "edit_params":
            result = edit_params(input_value["cases"])
        case "edit_request":
            result = edit_request(input_value["cases"])
        case "edit_reply":
            result = edit_reply(input_value["cases"])
    return result
