import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Protocol, TypeAlias, TypedDict, Unpack

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.types.utils import EmbeddingResponse
from openai import APIError, APIStatusError, AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue as Json
from pydantic import BaseModel, TypeAdapter

from azure_embeddings.report import (
    ClientName,
    EmbeddingInput,
    Input,
    Recorder,
    Step,
    azure_value,
    reply_value,
)

BODY = TypeAdapter(dict[str, Json])
ENDPOINT = "https://fixture.openai.azure.com"
V1_URL = "https://fixture.openai.azure.com/openai/v1/"
API_VERSION = "2024-02-01"
CLIENT_KEY = "fixture-key"
SdkClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI


class Session:
    def __init__(self, step: Step) -> None:
        self.answer = step["answer"]
        # the SDK adds encoding_format when none was sent, which docs/deviations/B534.yaml lists
        self.sdk_adds_format = "encoding_format" not in step["optional_params"]
        self.calls: list[Json] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        body = BODY.validate_json(request.content)
        if self.sdk_adds_format:
            body.pop("encoding_format", None)
        self.calls.append({"body": body})
        answer = self.answer
        if "connection" in answer:
            raise httpx.ConnectError("fixture connection failure", request=request)
        reply = answer["reply"]
        return httpx.Response(
            reply["status"],
            headers=reply["headers"],
            content=reply["body"].encode(),
            request=request,
        )


def make_client(name: ClientName, session: Session) -> SdkClient:
    transport = httpx.MockTransport(session.handle)
    match name:
        case "azure_open_ai":
            return AzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        case "async_azure_open_ai":
            return AsyncAzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )
        case "open_ai":
            return OpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        case "async_open_ai":
            return AsyncOpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )


class Configurable(Protocol):
    def update_environment_variables(
        self,
        litellm_params: dict[str, Callable[[dict[str, Json]], None]],
        optional_params: dict[str, Json],
        model: str | None = None,
    ) -> None: ...


class EmbeddingArgs(TypedDict):
    model: str
    input: EmbeddingInput
    api_base: str
    api_version: str
    timeout: float
    logging_obj: Logging
    model_response: EmbeddingResponse
    optional_params: dict[str, Json]
    api_key: str
    azure_ad_token: str | None
    max_retries: int | None
    client: SdkClient
    aembedding: bool
    headers: dict[str, str]
    litellm_params: dict[str, Json]


# the typed view of AzureChatCompletion.embedding, whose own signature has bare dict and list
class Embedder(Protocol):
    def embedding(
        self, **kwargs: Unpack[EmbeddingArgs]
    ) -> EmbeddingResponse | Awaitable[EmbeddingResponse]: ...


def logging_object(recorder: Recorder) -> Logging:
    logging_obj = Logging(
        model="my-deployment",
        messages=None,
        stream=False,
        call_type="embedding",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b534",
        function_id="b534",
    )
    configurable: Configurable = logging_obj
    logger_params: dict[str, Callable[[dict[str, Json]], None]] = {"logger_fn": recorder.log}
    no_params: dict[str, Json] = {}
    configurable.update_environment_variables(
        litellm_params=logger_params,
        optional_params=no_params,
        model="my-deployment",
    )
    return logging_obj


def call_embedding(step: Step, session: Session, logging_obj: Logging) -> EmbeddingResponse:
    embedder: Embedder = AzureChatCompletion()
    arguments = EmbeddingArgs(
        model=step["model"],
        input=step["input"],
        api_base=step["api_base"],
        api_version=step["api_version"],
        timeout=step["timeout"],
        logging_obj=logging_obj,
        model_response=EmbeddingResponse(),
        optional_params=BODY.validate_python(step["optional_params"]),
        api_key=step["api_key"],
        azure_ad_token=step["azure_ad_token"],
        max_retries=step["max_retries"],
        client=make_client(step["client"], session),
        aembedding=step["aembedding"],
        headers=step["headers"],
        litellm_params=BODY.validate_python(step["litellm_params"]),
    )
    outcome = embedder.embedding(**arguments)
    match outcome:
        case Awaitable():
            return asyncio.run(asyncio.wait_for(outcome, timeout=None))
        case BaseModel():
            return outcome


def raised_value(kind: str, status: int | None, message: str | None) -> dict[str, Json]:
    return {
        "kind": kind,
        "status": status,
        "message": message,
        "headers": None,
    }


def step_value(step: Step) -> dict[str, Json]:
    session = Session(step)
    recorder = Recorder()
    result: dict[str, Json] | None = None
    raised: dict[str, Json] | None = None
    try:
        result = reply_value(call_embedding(step, session, logging_object(recorder)))
    except ValueError as error:
        raised = raised_value("value", None, str(error))
    except APIStatusError as error:
        raised = raised_value("sdk_status", error.status_code, None)
    except APIError:
        raised = raised_value("sdk_other", None, None)
    except Exception as error:
        if type(error).__name__ != "AzureOpenAIError":
            raise
        raised = azure_value(error)
    return {
        "result": result,
        "raised": raised,
        "pre_call": recorder.pre,
        "post_call": recorder.post,
        "calls": session.calls,
    }


def run(input_value: Input) -> dict[str, Json]:
    # an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
    litellm.suppress_debug_info = True
    return {"steps": [step_value(step) for step in input_value["steps"]]}
