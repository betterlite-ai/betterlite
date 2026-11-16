import base64
import os
from dataclasses import dataclass
from io import BytesIO
from typing import Final

import httpx
import litellm
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue

from .configs import EDIT, GENERATION
from .inputs import (
    JSON,
    EditRequestCase,
    GenerationParamsCase,
    GenerationRequestCase,
    ImageCase,
    Input,
    Object,
    ReplyCase,
)
from .replies import REPLY, generation_fields, reply_fields

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
            dict(case["non_default_params"]),
            optional,
            case["model"],
            drop_params=case["drop_params"],
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
        raw = httpx.Response(case["status"], content=case["body"].encode(), headers=case["headers"])
        finished = GENERATION.transform_image_generation_response(
            case["model"], raw, response, None, {}, {}, {}, None
        )
        answers.append(generation_fields(finished))
    return JSON.validate_python(answers)


def edit_params(cases: list[Object]) -> JsonValue:
    supported = JSON.validate_python(EDIT.get_supported_openai_params(""))
    answers: list[JsonValue] = []
    for case in cases:
        mapped = EDIT.map_openai_params(dict(case), "", drop_params=False)
        answers.append({"supported": supported, "params": mapped})
    return JSON.validate_python(answers)


def edit_request(case: EditRequestCase) -> JsonValue:
    activate(case["environment"], case["global_api_key"])
    url = EDIT.get_complete_url(case["model"], case["api_base"], {})
    headers = EDIT.validate_environment(
        dict(case["headers"]), case["model"], case["api_key"], None, case["api_base"]
    )
    source = image_source(case["image"])
    request, files = EDIT.transform_image_edit_request(
        case["model"],
        case["prompt"],
        source.value,
        dict(case["optional_params"]),
        GenericLiteLLMParams(),
        headers,
    )
    return {
        "url": url,
        "headers": JSON.validate_python(headers),
        "request": JSON.validate_python(request),
        "files": JSON.validate_python(files),
        "position": source.position(),
    }


def edit_requests(cases: list[EditRequestCase]) -> JsonValue:
    answers = [edit_request(case) for case in cases]
    return JSON.validate_python(answers)


def edit_reply(cases: list[ReplyCase]) -> JsonValue:
    answers: list[JsonValue] = []
    for case in cases:
        raw = httpx.Response(case["status"], content=case["body"].encode(), headers=case["headers"])
        finished = EDIT.transform_image_edit_response(case["model"], raw, None)
        answers.append(reply_fields(REPLY.validate_json(finished.model_dump_json()), finished))
    return JSON.validate_python(answers)


def run(input_value: Input) -> JsonValue:
    match input_value["op"]:
        case "generation_params":
            return generation_params(input_value["cases"])
        case "generation_request":
            return generation_request(input_value["cases"])
        case "generation_reply":
            return generation_reply(input_value["cases"])
        case "edit_params":
            return edit_params(input_value["cases"])
        case "edit_request":
            return edit_requests(input_value["cases"])
        case "edit_reply":
            return edit_reply(input_value["cases"])
