import os

os.environ["LITELLM_RUST"] = "false"

import asyncio
from typing import Final

from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.responses.transformation import BedrockOpenAIResponsesConfig
from litellm.llms.bedrock.vector_stores.transformation import BedrockVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

from agentcore_search_reply import run as search_reply
from bedrock_services_passthrough import SIGNER as PASSTHROUGH_SIGNER
from bedrock_services_passthrough import collect_run, reply_run, url_run
from bedrock_services_realtime_transport import run as realtime_transport_run
from bedrock_services_realtime_transport.models import Input as RealtimeTransportInput
from bedrock_services_realtime_transport.models import Run as RealtimeTransportRun
from bedrock_services_responses.calls import Fixture
from bedrock_services_responses.handlers import option_cases
from bedrock_services_responses.models import Calls
from bedrock_services_search import CONFIG as SEARCH
from bedrock_services_vector_stores import Input as VectorInput
from bedrock_services_vector_stores import (
    RequestScenario,
    ResponseScenario,
    decoded_responses,
    retrieval_configuration,
)

from . import inputs

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
SEARCH_ENVIRONMENT: Final = (
    "AGENTCORE_SEARCH_TOOL_NAME", "AGENTCORE_GATEWAY_TOKEN", "AGENTCORE_GATEWAY_URL"
)

def error_row(kind: str, status: int | None) -> inputs.JsonObject:
    row: inputs.JsonObject = dict[str, JsonValue].fromkeys(inputs.KEYS)
    row["error"] = {"kind": kind, "status": status}
    return row


def responses_row(run: inputs.ResponsesRun) -> JsonValue:
    fixture = Fixture({})
    config = BedrockOpenAIResponsesConfig(
        fetch_image=fixture.fetch, async_fetch_image=fixture.fetch_async
    )
    return responses_request(config, run)


def responses_request(calls: Calls, run: inputs.ResponsesRun) -> JsonValue:
    api_key = run.get("api_key")
    try:
        source = inputs.RESPONSES_SOURCE.validate_python(run["input"])
        payload: JsonValue = [{"params": run["params"], "drop_params": False}]
        validated = inputs.MAPPED_RESULT.validate_python(option_cases(calls, payload))
        mapped = validated["results"][0]["params"]
        url = calls.get_complete_url(RESPONSES_HOST, {})
        params = GenericLiteLLMParams(api_key=api_key)
        headers = calls.validate_environment({}, run["model"], params)
        body = asyncio.run(
            calls.async_transform_responses_api_request(run["model"], source, mapped, params, {})
        )
    except (KeyError, TypeError, ValueError):
        return error_row("leaf", None)
    signed, _body = calls.sign_request(dict(headers), {}, body, url, api_key=api_key)
    row: inputs.JsonObject = dict[str, JsonValue].fromkeys(inputs.KEYS)
    row["request"] = inputs.request_view("POST", url, signed, body)
    row["status"] = run["status"]
    row["reply"] = JsonAdapter.validate_json(run["reply"])
    return row


def passthrough_row(run: inputs.PassthroughRun) -> JsonValue:
    region = run.get("region")
    if region is None:
        region = "us-east-1"
    try:
        view = inputs.URL_VIEW.validate_python(
            url_run({"model": PASSTHROUGH_MODEL, "endpoint": run["endpoint"], "region": region})
        )
    except ValueError:
        return error_row("leaf", None)
    params: inputs.JsonObject = {
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
    text = b"{}" if body is None else body
    row: inputs.JsonObject = dict[str, JsonValue].fromkeys(inputs.KEYS)
    row["request"] = inputs.request_view(
        "POST", view["url"], signed, JsonAdapter.validate_json(text)
    )
    row["endpoint_url"] = view["endpoint_url"]
    row["streaming"] = view["streaming"]
    row["status"] = run["status"]
    chunks = run.get("chunks")
    if chunks:
        data = bytes.fromhex("".join(chunks))
        collected = inputs.COLLECT_VIEW.validate_python(
            collect_run({"model": PASSTHROUGH_MODEL, "endpoint": run["endpoint"]}, data)
        )
        row["logged"] = collected["decoded"]
    else:
        logged = inputs.LOGGED_VIEW.validate_python(
            reply_run({"provider": "bedrock", "endpoint": run["endpoint"], "body": run["reply"]})
        )
        row["logged"] = logged["logged"]
    return row


def search_row(run: inputs.SearchRun) -> JsonValue:
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
    row: inputs.JsonObject = dict[str, JsonValue].fromkeys(inputs.KEYS)
    row["request"] = inputs.request_view("POST", GATEWAY, signed, JsonAdapter.validate_python(body))
    try:
        row["response"] = JsonAdapter.validate_python(
            search_reply({"status": run["status"], "headers": [], "body": run["reply"]})
        )
    except BedrockError as error:
        return error_row("reply", error.status_code)
    return row


def vector_row(run: inputs.VectorRun) -> JsonValue:
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
    try:
        calls = inputs.VECTOR_CALLS.validate_python(
            retrieval_configuration(BedrockVectorStoreConfig(), supplied)
        )
    except ValueError:
        return error_row("leaf", None)
    if not calls["calls"]:
        return error_row("leaf", None)
    call = calls["calls"][0]
    row: inputs.JsonObject = dict[str, JsonValue].fromkeys(inputs.KEYS)
    row["request"] = inputs.request_view(
        "POST", call["url"], {}, JsonAdapter.validate_python(call["body"])
    )
    decoded = inputs.VECTOR_RESULT.validate_python(
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


def realtime_row(run: inputs.RealtimeRun) -> JsonValue:
    region = run.get("region")
    if region is None:
        region = "us-east-1"
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
                region=region,
                committed=False,
                failure=None,
            )
        ]
    )
    result = inputs.REALTIME_RESULT.validate_python(realtime_transport_run(scenario))
    seen = result["runs"][0]
    row: inputs.JsonObject = dict[str, JsonValue].fromkeys(inputs.KEYS)
    row["response"] = {
        "outcome": seen["outcome"],
        "error_kind": seen["error_kind"],
        "status_code": seen["status_code"],
        "committed": seen["committed"],
    }
    return row


def run(input_value: inputs.Case) -> JsonValue:
    if input_value["case"] == "responses":
        return {"runs": [responses_row(item) for item in input_value["runs"]]}
    if input_value["case"] == "passthrough":
        return {"runs": [passthrough_row(item) for item in input_value["runs"]]}
    if input_value["case"] == "search":
        return {"runs": [search_row(item) for item in input_value["runs"]]}
    if input_value["case"] == "vector_retrieve":
        return {"runs": [vector_row(item) for item in input_value["runs"]]}
    return {"runs": [realtime_row(item) for item in input_value["runs"]]}
