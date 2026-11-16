import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.types.utils import EmbeddingResponse
from openai import APIError, APIStatusError, AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from azure_embeddings.report import (
    ClientName,
    EmbeddingInput,
    Input,
    Recorder,
    Step,
    azure_value,
    reply_value,
)


class ResponseState(TypedDict, total=False):
    status_code: int
    message: str


BODY = TypeAdapter(dict[str, Json])
RESPONSE_STATE = TypeAdapter(ResponseState)
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
        case "azure":
            return AzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        case "async_azure":
            return AsyncAzureOpenAI(
                api_key=CLIENT_KEY,
                azure_endpoint=ENDPOINT,
                api_version=API_VERSION,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )
        case "v1":
            return OpenAI(
                api_key=CLIENT_KEY,
                base_url=V1_URL,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        case "async_v1":
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


# the typed view of AzureChatCompletion.embedding, whose own signature has bare dict and list
class Embedder(Protocol):
    def embedding(  # noqa: PLR0913  # LiteLLM's own parameter list: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L744-L761
        self,
        *,
        model: str,
        input: EmbeddingInput,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L747
        api_base: str,
        api_version: str,
        timeout: float,
        logging_obj: Logging,
        model_response: EmbeddingResponse,
        optional_params: dict[str, Json],
        api_key: str | None = None,
        azure_ad_token: str | None = None,
        azure_ad_token_provider: None = None,
        max_retries: int | None = None,
        client: SdkClient | None = None,
        aembedding: bool | None = None,
        headers: dict[str, str] | None = None,
        litellm_params: dict[str, Json] | None = None,
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
    attach_logger(logging_obj, recorder)
    return logging_obj


# the parameter is typed as the protocol, so the checker reads LiteLLM's bare dict through it
def attach_logger(logging_obj: Configurable, recorder: Recorder) -> None:
    logger_params: dict[str, Callable[[dict[str, Json]], None]] = {"logger_fn": recorder.log}
    no_params: dict[str, Json] = {}
    logging_obj.update_environment_variables(
        litellm_params=logger_params,
        optional_params=no_params,
        model="my-deployment",
    )


def call_embedding(
    embedder: Embedder, step: Step, session: Session, logging_obj: Logging
) -> EmbeddingResponse:
    outcome = embedder.embedding(
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
    match outcome:
        case Awaitable():
            return asyncio.run(asyncio.wait_for(outcome, timeout=None))
        case EmbeddingResponse():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2298
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
        embedder = AzureChatCompletion()
        result = reply_value(call_embedding(embedder, step, session, logging_object(recorder)))
    except ValueError as error:
        raised = raised_value("value", None, str(error))
    except APIStatusError as error:
        raised = raised_value("sdk_status", error.status_code, None)
    except APIError:
        raised = raised_value("sdk_other", None, None)
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        raised = azure_value(error)
    except Exception as error:
        # the bare Exception of convert_to_model_response_object keeps status_code and message
        state = RESPONSE_STATE.validate_python(error.__getstate__())
        if "status_code" not in state or "message" not in state:
            raise
        raised = raised_value("response", state["status_code"], state["message"])
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
