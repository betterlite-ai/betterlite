import os

os.environ["LITELLM_RUST"] = "false"

import asyncio
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
from pydantic import JsonValue, TypeAdapter

from agentcore_search_reply import run as search_reply
from bedrock_services_passthrough import SIGNER as PASSTHROUGH_SIGNER, collect_run, logging_fixture, url_run
from bedrock_services_realtime_transport import run as realtime_transport_run
from bedrock_services_realtime_transport.models import Input as RealtimeTransportInput
from bedrock_services_realtime_transport.models import Run as RealtimeTransportRun
from bedrock_services_responses.calls import Fixture
from bedrock_services_responses.handlers import option_cases
from bedrock_services_responses.models import Calls, Input as ResponsesSource
from bedrock_services_search import CONFIG as SEARCH
from bedrock_services_vector_stores import (
    Input as VectorInput,
    RequestScenario,
    ResponseScenario,
    decoded_responses,
    retrieval_configuration,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.passthrough.transformation import BedrockPassthroughConfig
from litellm.llms.bedrock.responses.transformation import BedrockOpenAIResponsesConfig
from litellm.llms.bedrock.vector_stores.transformation import BedrockVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams

JsonObject = dict[str, JsonValue]
JsonAdapter: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

RESPONSES_HOST: Final = "https://bedrock-runtime.us-west-2.amazonaws.com"
GATEWAY: Final = "https://gateway.example.com/mcp"
VECTOR_ENDPOINT: Final = "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases"
VECTOR_ID: Final = "KB123"
PASSTHROUGH_MODEL: Final = "anthropic.claude-3"
REALTIME_MODEL: Final = "amazon.nova-sonic-v1:0"
ACCESS: Final = "AKIDBETTERLITEFIXTURE"
SEED: Final = "fixture/secret/key/+EXAMPLEKEY0000"
MODE: Final = "resolved"
SEARCH_ENVIRONMENT: Final = ("AGENTCORE_SEARCH_TOOL_NAME", "AGENTCORE_GATEWAY_TOKEN", "AGENTCORE_GATEWAY_URL")


class UrlView(TypedDict):
    url: str
    endpoint_url: str
    streaming: bool


class CollectView(TypedDict):
    decoded: bool


class MappedParams(TypedDict):
    params: JsonObject


class MappedResult(TypedDict):
    results: list[MappedParams]


class VectorCall(TypedDict):
    url: str
    body: JsonObject
    query: str


class VectorCalls(TypedDict):
    calls: list[VectorCall]


class VectorValue(TypedDict):
    value: JsonObject


class VectorResult(TypedDict):
    responses: list[VectorValue]


class RealtimeSeen(TypedDict):
    outcome: str
    error_kind: str | None
    status_code: int | None
    runtime_closed: bool
    committed: bool


class RealtimeResult(TypedDict):
    runs: list[RealtimeSeen]


URL_VIEW: Final[TypeAdapter[UrlView]] = TypeAdapter(UrlView)
COLLECT_VIEW: Final[TypeAdapter[CollectView]] = TypeAdapter(CollectView)
MAPPED_RESULT: Final[TypeAdapter[MappedResult]] = TypeAdapter(MappedResult)
VECTOR_CALLS: Final[TypeAdapter[VectorCalls]] = TypeAdapter(VectorCalls)
VECTOR_RESULT: Final[TypeAdapter[VectorResult]] = TypeAdapter(VectorResult)
REALTIME_RESULT: Final[TypeAdapter[RealtimeResult]] = TypeAdapter(RealtimeResult)
RESPONSES_SOURCE: Final[TypeAdapter[ResponsesSource]] = TypeAdapter(ResponsesSource)


class PassthroughLogging(Protocol):
    def logging_non_streaming_response(
        self,
        model: str,
        custom_llm_provider: str,
        httpx_response: httpx.Response,
        request_data: dict[str, JsonValue],
        logging_obj: Logging,
        endpoint: str,
    ) -> object | None: ...


LOGGING: Final[PassthroughLogging] = BedrockPassthroughConfig()


class ResponsesRun(TypedDict):
    model: str
    input: JsonValue
    params: JsonObject
    api_key: NotRequired[str | None]
    status: int
    reply: str


class PassthroughRun(TypedDict):
    endpoint: str
    data: JsonObject
    region: NotRequired[str | None]
    status: int
    reply: str
    chunks: NotRequired[list[str]]


class SearchRun(TypedDict):
    query: str
    optional: JsonObject
    tool_name: NotRequired[str | None]
    api_key: NotRequired[str | None]
    status: int
    reply: str


class VectorRun(TypedDict):
    query: str
    optional: JsonObject
    extra_body: NotRequired[JsonObject | None]
    status: int
    reply: str


class RealtimeRun(TypedDict):
    pending: str
    region: NotRequired[str | None]


class ResponsesCase(TypedDict):
    case: Literal["responses"]
    runs: list[ResponsesRun]


class PassthroughCase(TypedDict):
    case: Literal["passthrough"]
    runs: list[PassthroughRun]


class SearchCase(TypedDict):
    case: Literal["search"]
    runs: list[SearchRun]


class VectorCase(TypedDict):
    case: Literal["vector_retrieve"]
    runs: list[VectorRun]


class RealtimeCase(TypedDict):
    case: Literal["realtime_bridge"]
    runs: list[RealtimeRun]


Case = ResponsesCase | PassthroughCase | SearchCase | VectorCase | RealtimeCase

KEYS: Final = ("request", "endpoint_url", "streaming", "status", "reply", "response", "logged", "error")


def empty_row() -> JsonObject:
    return {key: None for key in KEYS}


def error_row(kind: str, status: int | None) -> JsonObject:
    row = empty_row()
    row["error"] = {"kind": kind, "status": status}
    return row


def authorization_view(headers: dict[str, str]) -> JsonValue:
    value = headers.get("Authorization")
    if value is None:
        return None
    if value.startswith("Bearer "):
        return {
            "kind": "bearer",
            "access_key": None,
            "region": None,
            "service": None,
            "token": value[len("Bearer ") :],
        }
    scope = value.partition("Credential=")[2].partition(",")[0]
    access_key, _date, region, service, _scope = scope.split("/")
    return {
        "kind": "sigv4",
        "access_key": access_key,
        "region": region,
        "service": service,
        "token": None,
    }


def request_view(method: str, url: str, headers: dict[str, str], body: JsonValue) -> JsonValue:
    return {
        "method": method,
        "url": url,
        "auth": authorization_view(headers),
        "body": body,
    }


def mapped_params(calls: Calls, params: JsonObject) -> JsonObject:
    payload: JsonValue = [{"params": params, "drop_params": False}]
    mapped = MAPPED_RESULT.validate_python(option_cases(calls, payload))
    return mapped["results"][0]["params"]


def responses_row(run: ResponsesRun) -> JsonValue:
    fixture = Fixture({})
    config = BedrockOpenAIResponsesConfig(
        fetch_image=fixture.fetch, async_fetch_image=fixture.fetch_async
    )
    calls: Calls = config
    api_key = run.get("api_key")
    try:
        source = RESPONSES_SOURCE.validate_python(run["input"])
        mapped = mapped_params(calls, run["params"])
        url = calls.get_complete_url(RESPONSES_HOST, {})
        params = GenericLiteLLMParams(api_key=api_key)
        headers = calls.validate_environment({}, run["model"], params)
        body = asyncio.run(
            calls.async_transform_responses_api_request(
                run["model"], source, mapped, params, {}
            )
        )
    except (KeyError, TypeError, ValueError):
        return error_row("leaf", None)
    signed, _body = calls.sign_request(dict(headers), {}, body, url, api_key=api_key)
    row = empty_row()
    row["request"] = request_view("POST", url, signed, body)
    row["status"] = run["status"]
    row["reply"] = JsonAdapter.validate_json(run["reply"])
    return row


def passthrough_logged(run: PassthroughRun) -> bool:
    if run.get("chunks"):
        data = bytes.fromhex("".join(run["chunks"]))
        collected = COLLECT_VIEW.validate_python(
            collect_run({"model": PASSTHROUGH_MODEL, "endpoint": run["endpoint"]}, data)
        )
        return collected["decoded"]
    response = httpx.Response(run["status"], content=run["reply"].encode())
    logged = LOGGING.logging_non_streaming_response(
        model=PASSTHROUGH_MODEL,
        custom_llm_provider="bedrock",
        httpx_response=response,
        request_data=run["data"],
        logging_obj=logging_fixture(PASSTHROUGH_MODEL),
        endpoint=run["endpoint"],
    )
    return logged is not None


def passthrough_row(run: PassthroughRun) -> JsonValue:
    region = run.get("region") or "us-east-1"
    try:
        view = URL_VIEW.validate_python(
            url_run({"model": PASSTHROUGH_MODEL, "endpoint": run["endpoint"], "region": region})
        )
    except ValueError:
        return error_row("leaf", None)
    params: JsonObject = {
        "aws_region_name": region,
        "aws_access_key_id": ACCESS,
        "aws_secret_access_key": SEED,
    }
    signed, body = PASSTHROUGH_SIGNER.sign_request(
        headers={},
        litellm_params=params,
        request_data=run["data"],
        api_base=view["url"],
        model=PASSTHROUGH_MODEL,
    )
    row = empty_row()
    row["request"] = request_view(
        "POST", view["url"], signed, JsonAdapter.validate_json(body if body is not None else b"{}")
    )
    row["endpoint_url"] = view["endpoint_url"]
    row["streaming"] = view["streaming"]
    row["status"] = run["status"]
    row["logged"] = passthrough_logged(run)
    return row


def search_row(run: SearchRun) -> JsonValue:
    for name in SEARCH_ENVIRONMENT:
        os.environ.pop(name, None)
    tool_name = run.get("tool_name")
    if tool_name is not None:
        os.environ["AGENTCORE_SEARCH_TOOL_NAME"] = tool_name
    try:
        optional = run["optional"]
        headers = SEARCH.validate_environment({})
        body = SEARCH.transform_search_request(run["query"], optional)
        api_key = run.get("api_key")
        signed, _body = SEARCH.sign_request(dict(headers), optional, body, GATEWAY, api_key)
    except ValueError:
        return error_row("leaf", None)
    finally:
        for name in SEARCH_ENVIRONMENT:
            os.environ.pop(name, None)
    row = empty_row()
    row["request"] = request_view("POST", GATEWAY, signed, JsonAdapter.validate_python(body))
    try:
        row["response"] = JsonAdapter.validate_python(
            search_reply({"status": run["status"], "headers": [], "body": run["reply"]})
        )
    except BedrockError as error:
        return error_row("reply", error.status_code)
    return row


def vector_row(run: VectorRun) -> JsonValue:
    supplied = VectorInput(
        operation="retrieval_configuration",
        requests=[
            RequestScenario(
                vector_store_id=VECTOR_ID,
                query=run["query"],
                api_base=VECTOR_ENDPOINT,
                optional_params=run["optional"],
                litellm_params={"aws_region_name": "us-east-1"},
                extra_body=run.get("extra_body"),
            )
        ],
    )
    calls = vector_calls(supplied)
    if calls is None:
        return error_row("leaf", None)
    call = calls["calls"][0]
    row = empty_row()
    row["request"] = request_view("POST", call["url"], {}, JsonAdapter.validate_python(call["body"]))
    decoded = VECTOR_RESULT.validate_python(
        decoded_responses(
            BedrockVectorStoreConfig(),
            VectorInput(
                operation="attributes_and_scores",
                responses=[
                    ResponseScenario(status=run["status"], body=run["reply"], query=call["query"])
                ],
            ),
        )
    )
    row["response"] = decoded["responses"][0]["value"]
    return row


def vector_calls(supplied: VectorInput) -> VectorCalls | None:
    raw = retrieval_configuration(BedrockVectorStoreConfig(), supplied)
    try:
        calls = VECTOR_CALLS.validate_python(raw)
    except ValueError:
        return None
    return calls if calls["calls"] else None


def realtime_row(run: RealtimeRun) -> JsonValue:
    scenario = RealtimeTransportInput(
        runs=[
            RealtimeTransportRun(
                model=REALTIME_MODEL,
                pending=run["pending"],
                frames=[],
                chunks=[],
                block="client",
                receive_failure=None,
                send_failure=None,
                open_error=True,
                client_send_failure=None,
                credentials=MODE,
                auth={
                    "access_key_id": ACCESS,
                    "secret_access_key": SEED,
                    "session_token": None,
                },
                api_base=None,
                endpoint=None,
                region=run.get("region") or "us-east-1",
                committed=False,
                failure=None,
            )
        ]
    )
    result = REALTIME_RESULT.validate_python(realtime_transport_run(scenario))
    seen = result["runs"][0]
    row = empty_row()
    row["response"] = {
        "outcome": seen["outcome"],
        "error_kind": seen["error_kind"],
        "status_code": seen["status_code"],
        "committed": seen["committed"],
    }
    return row


def run(input_value: Case) -> JsonValue:
    if input_value["case"] == "responses":
        return {"runs": [responses_row(item) for item in input_value["runs"]]}
    if input_value["case"] == "passthrough":
        return {"runs": [passthrough_row(item) for item in input_value["runs"]]}
    if input_value["case"] == "search":
        return {"runs": [search_row(item) for item in input_value["runs"]]}
    if input_value["case"] == "vector_retrieve":
        return {"runs": [vector_row(item) for item in input_value["runs"]]}
    return {"runs": [realtime_row(item) for item in input_value["runs"]]}
