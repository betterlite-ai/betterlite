import asyncio
import os
from collections.abc import Callable, Coroutine
from typing import Literal, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.rerank.handler import BedrockRerankHandler
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import RerankResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

JsonObject = dict[str, JsonValue]
Document = str | JsonObject
Function = Literal[
    "text_and_json_document_source",
    "rerank_model_arn",
    "top_n",
    "results_index_score",
    "return_document_content",
    "rerank_error",
    "rerank_transport_logging",
]
RerankCall = Callable[
    [
        str,
        str,
        list[Document],
        JsonObject,
        Logging,
        int | None,
        list[str] | None,
        bool | None,
        int | None,
        bool | None,
        float | None,
        str | None,
        JsonObject | None,
        HTTPHandler | AsyncHTTPHandler | None,
    ],
    RerankResponse | Coroutine[None, None, RerankResponse],
]


class Handler(Protocol):
    @property
    def rerank(self) -> RerankCall: ...


class Credentials(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    failure: str | None


class Probe(TypedDict):
    model: str
    query: str
    documents: list[Document]
    top_n: int | None
    rank_fields: list[str] | None
    return_documents: bool | None
    max_chunks_per_doc: int | None
    optional_params: JsonObject
    extra_headers: dict[str, str] | None
    api_base: str | None
    credentials: Credentials
    reply: Reply
    supplied: Literal["sync", "async"]
    is_async: bool
    timeout_ms: int | None


class Input(TypedDict):
    function: Function
    probes: list[Probe]


class RecordingLogging(Logging):
    def __init__(self) -> None:
        self.pre_input: JsonValue = None
        self.pre_api_base: JsonValue = None
        self.pre_complete_input_dict: JsonValue = None
        self.post_called = False

    @override
    def pre_call(
        self,
        input: JsonValue,
        api_key: str,
        model: str | None = None,
        additional_args: JsonObject | None = None,
    ) -> None:
        args: JsonObject = {} if additional_args is None else additional_args
        self.pre_input = input
        self.pre_api_base = args["api_base"]
        self.pre_complete_input_dict = args["complete_input_dict"]

    @override
    def post_call(
        self,
        original_response: JsonValue,
        input: JsonValue = None,
        api_key: str | None = None,
        additional_args: JsonObject | None = None,
    ) -> None:
        self.post_called = True


class Transport:
    def __init__(self, probe: Probe) -> None:
        self.probe = probe
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        reply = self.probe["reply"]
        failure = reply["failure"]
        if failure == "timeout":
            raise httpx.ReadTimeout("read timed out", request=request)
        if failure == "failed":
            raise httpx.ReadError("the transport failed", request=request)
        return httpx.Response(
            reply["status"], headers=reply["headers"], content=reply["body"].encode()
        )


def settle(pending: RerankResponse | Coroutine[None, None, RerankResponse]) -> RerankResponse:
    if asyncio.iscoroutine(pending):
        return asyncio.run(pending)
    return pending


def call_value(
    handler: Handler, probe: Probe, logging: RecordingLogging, transport: Transport
) -> RerankResponse:
    supplied = probe["supplied"]
    client: HTTPHandler | AsyncHTTPHandler
    if supplied == "sync":
        client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(transport)))
    else:
        client = AsyncHTTPHandler(transport=httpx.MockTransport(transport))
    extra_headers: JsonObject | None = (
        None
        if probe["extra_headers"] is None
        else dict[str, JsonValue](probe["extra_headers"])
    )
    timeout: float | None = None if probe["timeout_ms"] is None else probe["timeout_ms"] / 1000
    return settle(
        handler.rerank(
            probe["model"],
            probe["query"],
            probe["documents"],
            probe["optional_params"],
            logging,
            probe["top_n"],
            probe["rank_fields"],
            probe["return_documents"],
            probe["max_chunks_per_doc"],
            probe["is_async"],
            timeout,
            probe["api_base"],
            extra_headers,
            client,
        )
    )


def use_environment() -> None:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull


def prepare_value(probe: Probe) -> JsonValue:
    logging = RecordingLogging()
    transport = Transport(probe)
    call_value(BedrockRerankHandler(), probe, logging, transport)
    request = transport.requests[0]
    text = request.content.decode("utf-8")
    payload: JsonObject = {
        "posted_url": str(request.url),
        "posted_text": text,
        "posted_body": TypeAdapter(JsonValue).validate_json(text),
        "content_type": request.headers.get("content-type"),
        "signed_headers": {
            "authorization": "authorization" in request.headers,
            "amz_date": "x-amz-date" in request.headers,
        },
        "pre_call": {
            "input": logging.pre_input,
            "api_base": logging.pre_api_base,
            "complete_input_dict": logging.pre_complete_input_dict,
        },
    }
    return TypeAdapter(JsonValue).validate_python(payload)


def result_value(probe: Probe) -> JsonValue:
    response = call_value(BedrockRerankHandler(), probe, RecordingLogging(), Transport(probe))
    return TypeAdapter(JsonValue).validate_json(response.model_dump_json())


def error_value(probe: Probe) -> JsonValue:
    logging = RecordingLogging()
    status: int | None = None
    try:
        call_value(BedrockRerankHandler(), probe, logging, Transport(probe))
    except BedrockError as error:
        status = error.status_code
    except (httpx.HTTPError, TypeError, ValueError):
        status = None
    else:
        payload: JsonObject = {"ok": True, "status": None, "post_call": logging.post_called}
        return TypeAdapter(JsonValue).validate_python(payload)
    failure: JsonObject = {"ok": False, "status": status, "post_call": logging.post_called}
    return TypeAdapter(JsonValue).validate_python(failure)


def transport_value(probe: Probe) -> JsonValue:
    logging = RecordingLogging()
    transport = Transport(probe)
    call_value(BedrockRerankHandler(), probe, logging, transport)
    payload: JsonObject = {
        "posted_url": str(transport.requests[0].url),
        "timeout_ms": probe["timeout_ms"],
        "pre_call": logging.pre_input is not None,
        "post_call": logging.post_called,
    }
    return TypeAdapter(JsonValue).validate_python(payload)


HANDLERS: dict[Function, Callable[[Probe], JsonValue]] = {
    "text_and_json_document_source": prepare_value,
    "rerank_model_arn": prepare_value,
    "top_n": prepare_value,
    "return_document_content": prepare_value,
    "results_index_score": result_value,
    "rerank_error": error_value,
    "rerank_transport_logging": transport_value,
}


def run(input_value: Input) -> JsonValue:
    use_environment()
    handler_for = HANDLERS[input_value["function"]]
    values: list[JsonValue] = [handler_for(probe) for probe in input_value["probes"]]
    return TypeAdapter(JsonValue).validate_python(values)
