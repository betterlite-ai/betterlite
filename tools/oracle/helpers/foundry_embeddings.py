import asyncio
import json
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.embed import handler as embed_handler
from litellm.llms.azure_ai.embed.handler import AzureAIEmbedding
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import EmbeddingResponse
from openai import AsyncOpenAI, OpenAI
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

TEXT_BASE: Final = "https://text.test/v1"
# a fixed start time keeps the helper off the clock; Logging only stores it in its details
LOG_START: Final = datetime(1970, 1, 1, tzinfo=UTC)
BODY_KEYS: Final = frozenset({"model", "input"})


class HeaderCall(TypedDict):
    name: str
    value: str


class ReplyCall(TypedDict):
    status: int
    headers: list[HeaderCall]
    body: str


class FlowCall(TypedDict):
    mode: Literal["sync", "async"]
    model: str
    input: list[str]
    options: dict[str, JsonValue]
    api_base: str | None
    api_key: str | None
    timeout: int
    image_reply: ReplyCall
    text_reply: ReplyCall
    client: Literal["none", "openai", "handler"]


class Input(TypedDict):
    flows: list[FlowCall]


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyView(TypedDict):
    object: str
    model: str | None
    data: list[JsonValue]
    usage: UsageView


class TextBody(TypedDict):
    model: str
    input: list[JsonValue]


class AsyncParams(TypedDict):
    timeout: float


class TimeoutValues(BaseModel):
    read: float | None = None


class Extensions(BaseModel):
    model_config = ConfigDict(extra="ignore")
    timeout: TimeoutValues | None = None


# httpx records the client's timeout on each request it sends
class RequestFacts(BaseModel):
    model_config = ConfigDict(from_attributes=True, extra="ignore")
    extensions: Extensions


class HiddenFields(BaseModel):
    model_config = ConfigDict(extra="ignore")
    additional_headers: dict[str, JsonValue] | None = None


class HiddenModel(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    hidden: HiddenFields = Field(alias="_hidden_params")


# LiteLLM raises a bare Exception for an error inside a reply and sets status_code on it
class ReplyErrorFacts(Protocol):
    status_code: JsonValue


JSON_OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
REPLY: TypeAdapter[ReplyView] = TypeAdapter(ReplyView)
TEXT_BODY: TypeAdapter[TextBody] = TypeAdapter(TextBody)

Client: TypeAlias = HTTPHandler | AsyncHTTPHandler | OpenAI | AsyncOpenAI | None
Pending: TypeAlias = Coroutine[None, None, EmbeddingResponse]
EmbedCall: TypeAlias = Callable[
    [str, list[str], float, Logging, EmbeddingResponse, dict[str, JsonValue], str | None,
     str | None, Client, bool],
    EmbeddingResponse | Pending,
]


# LiteLLM declares EmbeddingResponse, but with aembedding=True the call returns the coroutine
class EmbeddingApi(Protocol):
    @property
    def embedding(self) -> EmbedCall: ...


class FlowState:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []
        self.constructed: list[JsonValue] = []


def text_call(request: httpx.Request) -> JsonValue:
    body = JSON_OBJECT.validate_json(request.content)
    text = TEXT_BODY.validate_json(request.content)
    options: dict[str, JsonValue] = {
        name: value for name, value in body.items() if name not in BODY_KEYS
    }
    return {"kind": "text", "model": text["model"], "input": text["input"], "options": options}


# image answers a client the caller hands in, built answers the sync client LiteLLM builds itself
class Transport(httpx.MockTransport):
    def __init__(
        self, state: FlowState, reply: ReplyCall, kind: Literal["image", "built", "text"]
    ) -> None:
        super().__init__(self.respond)
        self.state = state
        self.reply = reply
        self.kind = kind

    def respond(self, request: httpx.Request) -> httpx.Response:
        if self.kind == "text":
            self.state.calls.append(text_call(request))
        else:
            image: JsonValue = {
                "kind": "image",
                "url": str(request.url),
                "authorization": request.headers["authorization"],
                "content_type": request.headers["content-type"],
                "body": JSON_OBJECT.validate_json(request.content),
            }
            if self.kind == "built":
                timeout = RequestFacts.model_validate(request).extensions.timeout
                read = None if timeout is None else timeout.read
                self.state.constructed.append({"kind": "sync", "timeout": read})
            self.state.calls.append(image)
        headers = [(header["name"], header["value"]) for header in self.reply["headers"]]
        return httpx.Response(
            self.reply["status"], headers=headers, content=self.reply["body"], request=request
        )


def async_client(
    recorder: FlowState,
    transport: httpx.MockTransport,
    llm_provider: str,
    params: AsyncParams,
) -> AsyncHTTPHandler:
    del llm_provider
    recorder.constructed.append({"kind": "async", "timeout": params["timeout"]})
    return AsyncHTTPHandler(transport=transport, timeout=params["timeout"])


def build_client(flow: FlowCall, image: Transport, text: Transport) -> Client:
    kind = flow["client"]
    if kind == "none":
        return None
    if kind == "handler":
        if flow["mode"] == "sync":
            return HTTPHandler(client=httpx.Client(transport=image))
        return AsyncHTTPHandler(transport=image)
    if flow["mode"] == "sync":
        sync_http = httpx.Client(transport=text)
        return OpenAI(api_key="fixture-text-key", base_url=TEXT_BASE, http_client=sync_http)
    async_http = httpx.AsyncClient(transport=text)
    return AsyncOpenAI(api_key="fixture-text-key", base_url=TEXT_BASE, http_client=async_http)


def flow_response(flow: FlowCall, client: Client) -> EmbeddingResponse:
    log = Logging(model=flow["model"], messages=None, stream=False, call_type="embedding",
                  start_time=LOG_START, litellm_call_id="", function_id="embedding")
    api = cast(EmbeddingApi, AzureAIEmbedding())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/embed/handler.py#L207-L241
    pending = api.embedding(flow["model"], flow["input"], float(flow["timeout"]), log,
                            EmbeddingResponse(), flow["options"], flow["api_key"],
                            flow["api_base"], client, flow["mode"] == "async")
    if flow["mode"] == "async":
        return asyncio.run(cast(Pending, pending))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/embed/handler.py#L230-L241
    return cast(EmbeddingResponse, pending)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/embed/handler.py#L288-L294


def flow_outcome(flow: FlowCall) -> dict[str, JsonValue]:
    state = FlowState()
    image = Transport(state, flow["image_reply"], "image")
    text = Transport(state, flow["text_reply"], "text")
    litellm.sync_transport = Transport(state, flow["image_reply"], "built")  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1699
    embed_handler.get_async_httpx_client = partial(async_client, state, image)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/embed/handler.py#L77
    status: JsonValue = None
    try:
        response = flow_response(flow, build_client(flow, image, text))
    except httpx.HTTPStatusError as error:
        kind, status = "HTTPStatusError", error.response.status_code
    except json.JSONDecodeError:
        kind = "JSONDecodeError"
    except KeyError:
        kind = "KeyError"
    except IndexError:
        kind = "IndexError"
    except ValueError:
        kind = "ValueError"
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/embed/handler.py#L180
        kind = "Exception"
        # only the reply-error Exception carries status_code; the None Embeddings ones carry none
        if hasattr(error, "status_code"):
            status = cast(ReplyErrorFacts, error).status_code  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L614-L617
    else:
        return answer_outcome(state, response)
    finally:
        litellm.sync_transport = None  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1699
    return outcome(state, None, {"kind": kind, "status": status})


def outcome(
    state: FlowState,
    response: dict[str, JsonValue] | None,
    error: dict[str, JsonValue] | None,
) -> dict[str, JsonValue]:
    return {
        "calls": state.calls,
        "constructed": state.constructed,
        "response": response,
        "error": error,
    }


def answer_outcome(state: FlowState, response: EmbeddingResponse) -> dict[str, JsonValue]:
    view = REPLY.validate_json(response.model_dump_json())
    hidden = HiddenModel.model_validate(response).hidden
    usage: dict[str, JsonValue] = {
        "prompt_tokens": view["usage"]["prompt_tokens"],
        "completion_tokens": view["usage"]["completion_tokens"],
        "total_tokens": view["usage"]["total_tokens"],
    }
    # a call that performs no request gets no conversion, so nothing sets the headers
    additional_headers: dict[str, JsonValue] = {}
    if hidden.additional_headers is not None:
        additional_headers = hidden.additional_headers
    answer: dict[str, JsonValue] = {
        "object": view["object"],
        "model": view["model"],
        "data": view["data"],
        "usage": usage,
        "additional_headers": additional_headers,
    }
    return outcome(state, answer, None)


def run(input_value: Input) -> JsonValue:
    flows: list[JsonValue] = [flow_outcome(flow) for flow in input_value["flows"]]
    answer: dict[str, JsonValue] = {"flows": flows}
    return answer
