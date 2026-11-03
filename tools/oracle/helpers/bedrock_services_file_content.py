from typing import TypeAlias, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from pydantic import BaseModel, ConfigDict

from bedrock_settings import ErrorView

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")
    method: str
    url: str


class Response(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status_code: int
    body: list[int]
    headers: list[tuple[str, str]]
    request: Request


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    response: Response


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    raw = supplied.response
    request = httpx.Request(raw.request.method, raw.request.url)
    response = httpx.Response(
        raw.status_code, content=bytes(raw.body), headers=raw.headers, request=request
    )
    try:
        content = BedrockFilesConfig().transform_file_content_response(  # pyright: ignore[reportUnknownMemberType]
            raw_response=response, logging_obj=cast(Logging, None), litellm_params={}
        )
    except BedrockError as error:
        viewed_error = cast(ErrorView, error)
        error_response = viewed_error.response
        exception_headers = viewed_error.headers
        own_headers = (
            list(exception_headers.multi_items())
            if isinstance(exception_headers, httpx.Headers)
            else exception_headers
        )
        return cast(Json, {
            "ok": False,
            "error": {
                "status_code": viewed_error.status_code,
                "message": viewed_error.message,
                "headers": own_headers,
                "request": {
                    "method": viewed_error.request.method,
                    "url": str(viewed_error.request.url),
                },
                "response": {
                    "status_code": error_response.status_code,
                    "body": list(error_response.content),
                    "headers": list(error_response.headers.multi_items()),
                    "request": {
                        "method": error_response.request.method,
                        "url": str(error_response.request.url),
                    },
                },
            },
        })
    binary = content
    summary = binary.logging_summary()
    return cast(Json, {
        "ok": True,
        "response": {
            "status_code": binary.response.status_code,
            "body": list(binary.response.content),
            "headers": list(binary.response.headers.multi_items()),
            "request": {"method": request.method, "url": str(request.url)},
        },
        "hidden_params": {},
        "summary": {
            "object": summary["object"],
            "content_type": summary["content_type"],
            "num_bytes": summary["num_bytes"],
        },
    })
