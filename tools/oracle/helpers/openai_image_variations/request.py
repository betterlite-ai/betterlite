import email.policy
from email.message import EmailMessage
from email.parser import BytesParser
from typing import Final

import httpx
import litellm
import openai
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.image_variations.handler import OpenAIImageVariationsHandler
from litellm.llms.openai.image_variations.transformation import OpenAIImageVariationConfig
from litellm.types.utils import ImageResponse, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue as Json

from .probe import DOCUMENT, NOW, Probe, data_view, file_object
from .transform import empty_answer, supported_value

litellm.suppress_debug_info = True


def absent_config(model: str, provider: LlmProviders) -> OpenAIImageVariationConfig | None:
    return None


REAL_CONFIG: Final = ProviderConfigManager.get_provider_image_variation_config


class Upstream:
    def __init__(self, probe: Probe) -> None:
        self.probe = probe
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        match self.probe.failure:
            case "timeout":
                raise httpx.ReadTimeout("read timed out", request=request)
            case "failed":
                raise httpx.ReadError("the transport failed", request=request)
            case None:
                return httpx.Response(
                    self.probe.reply.status,
                    headers=self.probe.reply.headers,
                    content=self.probe.reply.body.encode(),
                )


def header_names(fields: dict[str, Json]) -> list[Json]:
    match fields.get("headers"):
        case dict() as headers:
            names: list[Json] = []
            for name in sorted(DOCUMENT.validate_python(headers)):
                names.append(name)
            return names
        case None | bool() | int() | float() | str() | list():
            return []


def body_view(value: Json, probe: Probe) -> Json:
    match value:
        case dict() as fields:
            typed = DOCUMENT.validate_python(fields)
            match typed.get("data"):
                case dict() as data:
                    return {"data": data_view(DOCUMENT.validate_python(data), probe)}
                case None | bool() | int() | float() | str() | list():
                    return None
        case None | bool() | int() | float() | str() | list():
            return None


def original_view(value: Json) -> Json:
    match value:
        case dict() as fields:
            return DOCUMENT.validate_python(fields)
        case None | bool() | int() | float() | str() | list():
            return None


def arguments_view(additional: Json, input_value: Json, original: Json, probe: Probe) -> Json:
    match additional:
        case dict() as fields:
            typed = DOCUMENT.validate_python(fields)
            return {
                "input": input_value,
                "api_base": typed.get("api_base"),
                "header_names": header_names(typed),
                "body": body_view(typed.get("complete_input_dict"), probe),
                "original_response": original_view(original),
            }
        case None | bool() | int() | float() | str() | list():
            raise TypeError("the logging additional args are an object")


class Recorder:
    def __init__(self, probe: Probe) -> None:
        self.probe = probe
        self.pre: list[Json] = []
        self.post: list[Json] = []

    def pre_call(self, **kwargs: Json) -> None:
        self.pre.append(
            arguments_view(
                kwargs["additional_args"], kwargs["input"], None, self.probe
            )
        )

    def post_call(self, **kwargs: Json) -> None:
        self.post.append(
            arguments_view(
                kwargs["additional_args"],
                kwargs.get("input"),
                kwargs["original_response"],
                self.probe,
            )
        )


def supplied_client(
    probe: Probe, transport: httpx.MockTransport
) -> openai.OpenAI | None:
    match probe.client:
        case "sync":
            return openai.OpenAI(
                api_key="sk-supplied",
                base_url="https://supplied.test/v1",
                organization="org-supplied",
                timeout=45.0,
                max_retries=1,
                http_client=httpx.Client(transport=transport),
            )
        case "async" | None:
            return None


def call_view(request: httpx.Request) -> Json:
    return {
        "url": str(request.url),
        "authorization": request.headers.get("authorization"),
        "organization": request.headers.get("openai-organization"),
        "parts": parts_view(request),
    }


def parts_view(request: httpx.Request) -> list[Json]:
    content_type = request.headers.get("content-type")
    if content_type is None:
        raise ValueError("a multipart request carries a content type")
    raw = (
        b"Content-Type: "
        + content_type.encode()
        + b"\r\nMIME-Version: 1.0\r\n\r\n"
        + request.read()
    )
    message = BytesParser(policy=email.policy.default).parsebytes(raw)
    return [part_view(part) for part in message.iter_parts()]


def part_view(part: EmailMessage) -> Json:
    match part.get_payload(decode=True):
        case bytes() as payload:
            data = payload.decode("utf-8")
        case None | str():
            raise TypeError("a multipart part carries bytes")
    return {
        "name": part.get_param("name", header="content-disposition"),
        "filename": part.get_filename(),
        "content_type": part.get("content-type"),
        "data": data,
    }


def error_headers(failure: OpenAIError, probe: Probe) -> Json:
    headers = failure.headers
    if headers is None:
        return None
    found: dict[str, Json] = {}
    for name, value in headers.items():
        if name.lower() in {key.lower() for key in probe.reply.headers}:
            found[name.lower()] = value
    return found


def diagnostic(failure: OpenAIError) -> Json:
    match failure.message:
        case message if "provider not found" in message:
            return "provider"
        case message if "data field is required" in message:
            return "data"
        case _:
            return None


def invoke(probe: Probe, client: openai.OpenAI | None, recorder: Recorder) -> ImageResponse:
    return OpenAIImageVariationsHandler().image_variations(
        model_response=ImageResponse(created=NOW),
        api_key=probe.api_key or "",
        api_base=probe.api_base or "",
        model=probe.model,
        image=file_object(probe),
        timeout=float(probe.timeout) if probe.timeout is not None else None,
        custom_llm_provider="openai",
        logging_obj=recorder,
        optional_params=dict(probe.optional_params),
        litellm_params={},
        client=client,
        organization=probe.organization,
        headers=probe.headers,
    )


def request_value(probe: Probe) -> Json:
    upstream = Upstream(probe)
    transport = httpx.MockTransport(upstream.handle)
    recorder = Recorder(probe)
    client = supplied_client(probe, transport)
    previous = litellm.client_session
    if client is None:
        litellm.client_session = httpx.Client(transport=transport)
    if probe.missing_config:
        ProviderConfigManager.get_provider_image_variation_config = absent_config
    error: Json = None
    result: Json = None
    try:
        response = invoke(probe, client, recorder)
    except OpenAIError as failure:
        error = {
            "mapped": True,
            "status": failure.status_code,
            "headers": error_headers(failure, probe),
            "diagnostic": diagnostic(failure),
        }
    else:
        result = DOCUMENT.validate_python(response.model_dump())
    finally:
        litellm.client_session = previous
        ProviderConfigManager.get_provider_image_variation_config = REAL_CONFIG
    answer = empty_answer(supported_value(OpenAIImageVariationConfig(), probe.model or ""))
    answer["supplied"] = probe.client is not None
    answer["kind"] = "sync"
    answer["calls"] = [call_view(request) for request in upstream.seen]
    answer["result"] = result
    answer["error"] = error
    answer["pre_call"] = recorder.pre[-1] if recorder.pre else None
    answer["post_call"] = recorder.post[-1] if recorder.post else None
    return answer
