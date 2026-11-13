import asyncio
import os
from typing import Literal, TypedDict

import httpx
from litellm.caching.caching import DualCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.dynamoai.dynamoai import DynamoAIGuardrails
from litellm.types.proxy.guardrails.guardrail_hooks.dynamoai import DynamoAIResponse
from litellm.types.utils import Choices, GuardrailStatus, ModelResponse
from pydantic import JsonValue, TypeAdapter

BODY_ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

CONTENT_TYPE_HEADER = "content-type"
AUTHORIZATION_HEADER = "authorization"
CLEARED_ENVIRONMENT = (
    "DYNAMOAI_API_KEY",
    "DYNAMOAI_API_BASE",
    "DYNAMOAI_POLICY_IDS",
    "DYNAMOAI_MODEL_ID",
)

Mode = Literal["constructor", "pre_call", "during_call", "post_call"]


class Choice(TypedDict):
    role: str
    content: str | None
    index: int


class Answer(TypedDict):
    status: int
    body: DynamoAIResponse


class RequestRecord(TypedDict):
    path: str
    content_type: str | None
    authorization_present: bool
    body: JsonValue


class Output(TypedDict):
    created: bool
    api_base: str | None
    policy_ids: list[str]
    model_id: str
    event_hooks: list[str]
    requests: list[RequestRecord]
    status: str | None
    raised: bool
    error: str | None


class Input(TypedDict):
    mode: Mode
    env: dict[str, str]
    api_key: str | None
    api_base: str | None
    policy_ids: list[str] | None
    model_id: str | None
    messages: list[dict[str, str]]
    choices: list[Choice]
    answer: Answer


class Script:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.calls: list[RequestRecord] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "path": request.url.path,
                "content_type": request.headers.get(CONTENT_TYPE_HEADER),
                "authorization_present": AUTHORIZATION_HEADER in request.headers,
                "body": BODY_ADAPTER.validate_json(request.content),
            }
        )
        return httpx.Response(self.answer["status"], json=self.answer["body"], request=request)


def build_guardrail(call: Input) -> DynamoAIGuardrails:
    for name in CLEARED_ENVIRONMENT:
        if name in os.environ:
            del os.environ[name]
    os.environ.update(call["env"])
    policy_ids: list[str] = [] if call["policy_ids"] is None else call["policy_ids"]
    model_id = "" if call["model_id"] is None else call["model_id"]
    return DynamoAIGuardrails(
        guardrail_name="dynamoai",
        api_key=call["api_key"],
        api_base=call["api_base"],
        model_id=model_id,
        policy_ids=policy_ids,
        default_on=True,
    )


def build_response(choices: list[Choice]) -> ModelResponse:
    built = [
        Choices(
            message={"role": choice["role"], "content": choice["content"]},
            index=choice["index"],
            finish_reason="stop",
        )
        for choice in choices
    ]
    return ModelResponse(choices=built)


def call_hook(
    guardrail: DynamoAIGuardrails,
    call: Input,
    data: dict[str, list[dict[str, str]]],
) -> None:
    if call["mode"] == "pre_call":
        asyncio.run(
            guardrail.async_pre_call_hook(
                user_api_key_dict=UserAPIKeyAuth(),
                cache=DualCache(),
                data=data,
                call_type="completion",
            )
        )
    elif call["mode"] == "during_call":
        asyncio.run(
            guardrail.async_moderation_hook(
                data=data,
                user_api_key_dict=UserAPIKeyAuth(),
                call_type="completion",
            )
        )
    else:
        asyncio.run(
            guardrail.async_post_call_success_hook(
                data=data,
                user_api_key_dict=UserAPIKeyAuth(),
                response=build_response(call["choices"]),
            )
        )


def run(input_value: Input) -> Output:
    try:
        guardrail = build_guardrail(input_value)
    except ValueError:
        return {
            "created": False,
            "api_base": None,
            "policy_ids": [],
            "model_id": "",
            "event_hooks": [],
            "requests": [],
            "status": None,
            "raised": False,
            "error": "missing_api_key",
        }
    script = Script(input_value["answer"])
    guardrail.async_handler = AsyncHTTPHandler(transport=httpx.MockTransport(script))
    raised = False
    if input_value["mode"] != "constructor":
        try:
            call_hook(guardrail, input_value, {"messages": input_value["messages"]})
        except ValueError:
            raised = True
    hooks = [hook.value for hook in DynamoAIGuardrails.get_supported_event_hooks()]
    status: GuardrailStatus | None = None
    if script.calls:
        body = input_value["answer"]["body"]
        status = guardrail._determine_guardrail_status(body)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/dynamoai/dynamoai.py#L224
    return {
        "created": True,
        "api_base": guardrail.api_base,
        "policy_ids": list(guardrail.policy_ids),
        "model_id": guardrail.model_id,
        "event_hooks": hooks,
        "requests": script.calls,
        "status": status,
        "raised": raised,
        "error": None,
    }
