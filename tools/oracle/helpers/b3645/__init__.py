from typing import Literal, TypeAlias, TypedDict

import httpx
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    transcribe_passthrough_logging_handler as handler,
)


class ErrorTypeInput(TypedDict):
    function: Literal["error_type"]
    body: str


class S3UrlInput(TypedDict):
    function: Literal["s3_url"]
    media_uri: str
    region: str


class PredatesInput(TypedDict):
    function: Literal["predates"]
    headers: dict[str, str]
    job_created_at: float


Input: TypeAlias = ErrorTypeInput | S3UrlInput | PredatesInput


def run(input_value: Input) -> str | bool | None:
    if input_value["function"] == "error_type":
        return handler.aws_error_type(
            httpx.Response(400, content=input_value["body"].encode())
        )
    if input_value["function"] == "s3_url":
        return handler.s3_media_url(input_value["media_uri"], input_value["region"])
    return handler.media_predates_job(
        httpx.Headers(input_value["headers"]), input_value["job_created_at"]
    )
