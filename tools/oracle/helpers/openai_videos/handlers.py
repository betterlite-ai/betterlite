import base64
import os
from collections.abc import Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Final

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from .config import API_BASE, CONFIG, ENV_NAME, MODEL, CreateParams
from .types import (
    VIDEO_LIST,
    Call,
    EnvironmentCall,
    Reference,
    VideoCreateOps,
    VideoEnvOps,
    VideoReplyOps,
    VideoRequestOps,
)
from .views import files_json, list_view, request_view, video_view

START: Final = datetime(2026, 1, 1, tzinfo=UTC)


def logging_object() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type="video_generation",
        start_time=START,
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def fixture_response(body_base64: str) -> httpx.Response:
    content = base64.b64decode(body_base64)
    request = httpx.Request("POST", API_BASE)
    return httpx.Response(200, content=content, request=request)


@contextmanager
def environment(call: EnvironmentCall) -> Generator[None]:
    previous_key = os.environ.get(ENV_NAME)
    previous_global = litellm.api_key
    previous_openai = litellm.openai_key
    litellm.api_key = call["global_api_key"]
    litellm.openai_key = call["openai_key"]
    if call["environment_api_key"] is None:
        os.environ.pop(ENV_NAME, None)
    else:
        os.environ[ENV_NAME] = call["environment_api_key"]
    try:
        yield
    finally:
        litellm.api_key = previous_global
        litellm.openai_key = previous_openai
        if previous_key is None:
            os.environ.pop(ENV_NAME, None)
        else:
            os.environ[ENV_NAME] = previous_key


def reference_content(reference: Reference) -> str | bytes:
    match reference["kind"]:
        case "text":
            return reference["text"]
        case "bytes":
            return base64.b64decode(reference["data_base64"])


def create_ops(call: VideoCreateOps) -> JsonValue:
    match call["op"]:
        case "create":
            params: CreateParams = dict(call["options"])
            reference = call["reference"]
            if reference is not None:
                params["input_reference"] = reference_content(reference)
            data, files, api_base = CONFIG.transform_video_create_request(
                call["model"], call["prompt"], call["api_base"], params, GenericLiteLLMParams(), {}
            )
            result: dict[str, JsonValue] = {
                "supported": CONFIG.get_supported_openai_params(call["model"]),
                "data": data,
                "files": files_json(files, reference),
                "api_base": api_base,
                "multipart": CONFIG.use_multipart_form_data(),
            }
            return result
        case "create_response":
            video = CONFIG.transform_video_create_response(
                call["model"],
                fixture_response(call["body_base64"]),
                logging_object(),
                call["provider"],
                None,
            )
            return video_view(video)


def request_ops(call: VideoRequestOps) -> JsonValue:
    match call["op"]:
        case "content_request":
            url, params = CONFIG.transform_video_content_request(
                call["video_id"], call["api_base"], GenericLiteLLMParams(), {}, call["variant"]
            )
            return request_view(url, params)
        case "remix_request":
            url, params = CONFIG.transform_video_remix_request(
                call["video_id"], call["prompt"], call["api_base"], GenericLiteLLMParams(), {}, None
            )
            return request_view(url, params)
        case "list_request":
            url, params = CONFIG.transform_video_list_request(
                call["api_base"],
                GenericLiteLLMParams(),
                {},
                call["after"],
                call["limit"],
                call["order"],
                None,
            )
            return request_view(url, params)
        case "delete_request":
            url, params = CONFIG.transform_video_delete_request(
                call["video_id"], call["api_base"], GenericLiteLLMParams(), {}
            )
            return request_view(url, params)
        case "status_request":
            url, params = CONFIG.transform_video_status_retrieve_request(
                call["video_id"], call["api_base"], GenericLiteLLMParams(), {}
            )
            return request_view(url, params)


def reply_ops(call: VideoReplyOps) -> JsonValue:
    match call["op"]:
        case "content_response":
            content = CONFIG.transform_video_content_response(
                fixture_response(call["body_base64"]), logging_object()
            )
            return {"body_base64": base64.b64encode(content).decode("ascii")}
        case "remix_response":
            video = CONFIG.transform_video_remix_response(
                fixture_response(call["body_base64"]), logging_object(), call["provider"]
            )
            return video_view(video)
        case "list_response":
            reply = CONFIG.transform_video_list_response(
                fixture_response(call["body_base64"]), logging_object(), call["provider"]
            )
            return list_view(VIDEO_LIST.validate_python(reply))
        case "delete_response":
            video = CONFIG.transform_video_delete_response(
                fixture_response(call["body_base64"]), logging_object()
            )
            return video_view(video)
        case "status_response":
            video = CONFIG.transform_video_status_retrieve_response(
                fixture_response(call["body_base64"]), logging_object(), call["provider"], None
            )
            return video_view(video)


def env_ops(call: VideoEnvOps) -> JsonValue:
    match call["op"]:
        case "validate_environment":
            params = GenericLiteLLMParams(api_key=call["params_api_key"])
            with environment(call):
                headers = CONFIG.validate_environment(
                    dict(call["headers"]), MODEL, call["api_key"], params
                )
            return {"headers": headers}
        case "complete_url":
            return {"url": CONFIG.get_complete_url(MODEL, call["api_base"], {})}
        case "error_class":
            raise CONFIG.get_error_class(call["message"], call["status"], call["headers"])


def run_call(call: Call) -> JsonValue:
    match call["op"]:
        case "create" | "create_response":
            return create_ops(call)
        case (
            "content_request" | "remix_request" | "list_request" | "delete_request"
            | "status_request"
        ):
            return request_ops(call)
        case (
            "content_response" | "remix_response" | "list_response" | "delete_response"
            | "status_response"
        ):
            return reply_ops(call)
        case "validate_environment" | "complete_url" | "error_class":
            return env_ops(call)
