import base64
from datetime import UTC, datetime
from typing import Protocol, runtime_checkable

import httpx
import litellm
import litellm.llms.runwayml.text_to_speech.transformation as runway_transformation
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import ConfigDict, JsonValue, TypeAdapter

from .fakes import Clock, Recorder, Scripted
from .inputs import DispatchInput, EMPTY, MappingInput, Object, RequestInput, ResponseInput
from .speech_config import CONFIG, JSON

SUBMISSION_URL = "https://submission.example.test/v1/text_to_speech"


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: HTTPHandler, *, ttl: int) -> None: ...


CLIENT_CACHE: TypeAdapter[ClientCache] = TypeAdapter(
    ClientCache, config=ConfigDict(arbitrary_types_allowed=True)
)


def logging_object(model: str) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=False,
        call_type="speech",
        start_time=datetime.now(UTC),
        litellm_call_id="runway-speech-call",
        function_id="runway-speech-function",
    )


def header_text(headers: httpx.Headers, name: str) -> str:
    value = headers.get(name)
    if value is None:
        return ""
    return value


def request_value(request: httpx.Request) -> JsonValue:
    headers: Object = {
        "authorization": header_text(request.headers, "authorization"),
        "x-runway-version": header_text(request.headers, "x-runway-version"),
    }
    answer: Object = {"url": str(request.url), "headers": headers}
    return JSON.validate_python(answer)


def response_value(finished: HttpxBinaryResponseContent) -> JsonValue:
    reply = finished.response
    answer: Object = {
        "status": reply.status_code,
        "content_type": reply.headers.get("content-type"),
        "body_b64": base64.b64encode(reply.content).decode("ascii"),
    }
    return JSON.validate_python(answer)


def mapping_value(input_value: MappingInput) -> JsonValue:
    params = input_value["params"]
    results: list[JsonValue] = []
    for call in params["calls"]:
        voice, mapped = CONFIG.map_openai_params(
            params["model"], call["optional_params"], call["voice"], call["drop_params"], EMPTY
        )
        results.append({"voice": voice, "params": mapped})
    answer: Object = {
        "supported": JSON.validate_python(CONFIG.get_supported_openai_params(params["model"])),
        "results": results,
    }
    return JSON.validate_python(answer)


def request_answer(input_value: RequestInput) -> JsonValue:
    params = input_value["params"]
    data = CONFIG.transform_text_to_speech_request(
        params["model"], params["input"], None, params["optional_params"], EMPTY, params["headers"]
    )
    answer: Object = {"dict_body": data["dict_body"], "headers": data["headers"]}
    return JSON.validate_python(answer)


def dispatch_value(input_value: DispatchInput) -> JsonValue:
    params = input_value["params"]
    globals_value = params["globals"]
    litellm.api_base = globals_value["api_base"]
    litellm.api_key = globals_value["api_key"]
    recorder = Recorder()
    CONFIG.dispatch_text_to_speech(
        params["model"],
        params["input"],
        params["voice"],
        params["optional_params"],
        dict(params["litellm_params"]),
        logging_object(params["model"]),
        params["timeout_seconds"],
        params["extra_headers"],
        recorder,
        params["aspeech"],
        params["api_base"],
        params["api_key"],
    )
    return JSON.validate_python(recorder.call)


def transform_value(input_value: ResponseInput) -> JsonValue:
    params = input_value["params"]
    scripted = Scripted(params["replies"])
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(scripted)))
    CLIENT_CACHE.validate_python(holder).set_cache(key="httpx_client", value=client, ttl=600)
    clock = Clock(params["step_millis"])
    object.__setattr__(runway_transformation, "time", clock)
    submission = params["submission"]
    request = httpx.Request("POST", SUBMISSION_URL, headers=params["request_headers"])
    raw_response = httpx.Response(
        submission["status"],
        headers=submission["headers"],
        content=submission["body"].encode(),
        request=request,
    )
    finished = CONFIG.transform_text_to_speech_response(
        params["model"], raw_response, logging_object(params["model"])
    )
    gets: list[JsonValue] = [request_value(item) for item in scripted.requests]
    answer: Object = {
        "response": response_value(finished),
        "gets": gets,
        "sleeps": clock.sleeps,
    }
    return JSON.validate_python(answer)
