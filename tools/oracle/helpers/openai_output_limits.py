import asyncio
import datetime as dt
from collections.abc import Coroutine, Iterator
from typing import Literal, NotRequired, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.openai.common_utils import (
    build_output_token_limit_response,
    drop_params_from_unprocessable_entity_error,
    is_output_token_limit_error,
)
from openai import AsyncAzureOpenAI, AzureOpenAI, BadRequestError, UnprocessableEntityError
from openai.types import CompletionUsage
from openai.types.chat import ChatCompletion, ChatCompletionChunk
from pydantic import JsonValue, TypeAdapter

REQUEST = httpx.Request("POST", "https://oracle.test/v1/chat/completions")

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
TEXT: TypeAdapter[str] = TypeAdapter(str)


class DropFields(TypedDict):
    channel: Literal["http", "sdk"]
    body: JsonValue
    data: dict[str, JsonValue]


class MarkerFields(TypedDict):
    message: str


class BuildFields(TypedDict):
    data: dict[str, JsonValue]
    headers: dict[str, str]


class Outcome(TypedDict):
    kind: Literal["reply", "bad_request", "status", "timeout"]
    status: NotRequired[int]
    headers: NotRequired[dict[str, str]]
    body: NotRequired[JsonValue]


class AzureFields(TypedDict):
    asynchronous: bool
    data: dict[str, JsonValue]
    timeout: float
    outcome: Outcome


DROP: TypeAdapter[DropFields] = TypeAdapter(DropFields)
MARKER: TypeAdapter[MarkerFields] = TypeAdapter(MarkerFields)
BUILD: TypeAdapter[BuildFields] = TypeAdapter(BuildFields)
AZURE: TypeAdapter[AzureFields] = TypeAdapter(AzureFields)


class Scripted:
    def __init__(self, outcome: Outcome) -> None:
        self.outcome = outcome
        self.bodies: list[JsonValue] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.bodies.append(JSON.validate_json(request.content))
        match self.outcome["kind"]:
            case "reply":
                return httpx.Response(
                    200,
                    headers=self.outcome.get("headers"),
                    json=self.outcome.get("body"),
                    request=request,
                )
            case "bad_request":
                return httpx.Response(
                    400,
                    headers=self.outcome.get("headers"),
                    json=self.outcome.get("body"),
                    request=request,
                )
            case "status":
                status = self.outcome.get("status")
                if status is None:
                    raise ValueError("the status outcome needs a status")
                return httpx.Response(
                    status,
                    headers=self.outcome.get("headers"),
                    json=self.outcome.get("body"),
                    request=request,
                )
            case "timeout":
                raise httpx.ReadTimeout("timed out", request=request)


def drop_value(fields: DropFields) -> JsonValue:
    body = fields["body"]
    raised: httpx.HTTPStatusError | UnprocessableEntityError
    match fields["channel"]:
        case "http":
            response = httpx.Response(422, text=TEXT.validate_python(body), request=REQUEST)
            raised = httpx.HTTPStatusError("rejected", request=REQUEST, response=response)
        case "sdk":
            response = httpx.Response(422, json=body, request=REQUEST)
            raised = UnprocessableEntityError("rejected", response=response, body=body)
    dropped = drop_params_from_unprocessable_entity_error(raised, fields["data"])
    return JSON.validate_python(dropped)


def marker_value(fields: MarkerFields) -> JsonValue:
    response = httpx.Response(400, request=REQUEST)
    error = BadRequestError(fields["message"], response=response, body=None)
    return is_output_token_limit_error(error)


ChoiceFields = tuple[int, str | None, str | None, str | None]


def projected_usage(usage: CompletionUsage) -> JsonValue:
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }


def projected_reply(
    object_name: str,
    model: str,
    choices: list[ChoiceFields],
    usage: CompletionUsage | None,
    body_key: Literal["message", "delta"],
) -> JsonValue:
    return {
        "object": object_name,
        "model": model,
        "choices": [
            {
                "index": index,
                "finish_reason": finish_reason,
                body_key: {"role": role, "content": content},
            }
            for index, finish_reason, role, content in choices
        ],
        "usage": None if usage is None else projected_usage(usage),
    }


def projected_completion(completion: ChatCompletion) -> JsonValue:
    choices: list[ChoiceFields] = [
        (choice.index, choice.finish_reason, choice.message.role, choice.message.content)
        for choice in completion.choices
    ]
    return projected_reply(
        completion.object, completion.model, choices, completion.usage, "message"
    )


def projected_chunk(chunk: ChatCompletionChunk) -> JsonValue:
    choices: list[ChoiceFields] = [
        (choice.index, choice.finish_reason, choice.delta.role, choice.delta.content)
        for choice in chunk.choices
    ]
    return projected_reply(chunk.object, chunk.model, choices, chunk.usage, "delta")


def build_value(fields: BuildFields) -> JsonValue:
    error = BadRequestError(
        "output limit",
        response=httpx.Response(400, headers=fields["headers"], request=REQUEST),
        body=None,
    )
    headers, result = build_output_token_limit_response(error, fields["data"], is_async=False)
    if fields["data"].get("stream"):
        chunks = cast("Iterator[ChatCompletionChunk]", result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L181-L203
        body = projected_chunk(next(chunks))
    else:
        completion = cast("ChatCompletion", result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L189-L203
        body = projected_completion(completion)
    return {"header": dict(headers).get("x-request-id"), "body": body}


def logging_object() -> Logging:
    no_messages: list[dict[str, str]] = []
    return Logging(
        model="gpt-3.5-turbo",
        messages=no_messages,
        stream=False,
        call_type="completion",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id="output-limits",
        function_id="output-limits",
    )


def azure_value(fields: AzureFields) -> JsonValue:
    scripted = Scripted(fields["outcome"])
    data: dict[str, JsonValue] = dict(fields["data"])
    headers: dict[str, str]
    reply: ChatCompletion
    if fields["asynchronous"]:
        async_client = AsyncAzureOpenAI(
            api_key="oracle-key",
            azure_endpoint="https://oracle.test",
            api_version="2024-02-01",
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(scripted.respond)),
            max_retries=0,
        )
        called = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L164-L194
            "Coroutine[None, None, tuple[dict[str, str], ChatCompletion]]",
            AzureChatCompletion().make_azure_openai_chat_completion_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L164-L194
                async_client,
                data,
                fields["timeout"],
                logging_obj=logging_object(),
            ),
        )
        headers, reply = asyncio.run(called)
    else:
        client = AzureOpenAI(
            api_key="oracle-key",
            azure_endpoint="https://oracle.test",
            api_version="2024-02-01",
            http_client=httpx.Client(transport=httpx.MockTransport(scripted.respond)),
            max_retries=0,
        )
        headers, reply = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L140-L162
            "tuple[dict[str, str], ChatCompletion]",
            AzureChatCompletion().make_sync_azure_openai_chat_completion_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L140-L162
                client, data, fields["timeout"]
            ),
        )
    return {
        "requests": scripted.bodies,
        "header": dict(headers).get("x-ms-request-id"),
        "body": projected_completion(reply),
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    operation = next(iter(input_value))
    fields = input_value[operation]
    match operation:
        case "drop":
            return drop_value(DROP.validate_python(fields))
        case "marker":
            return marker_value(MARKER.validate_python(fields))
        case "build":
            return build_value(BUILD.validate_python(fields))
        case "azure":
            return azure_value(AZURE.validate_python(fields))
        case _:
            raise ValueError(f"unsupported operation {operation}")
