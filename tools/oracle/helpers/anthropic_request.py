import asyncio
import logging
from collections.abc import Callable, Coroutine
from typing import Final, TypedDict

import litellm
from litellm.exceptions import BadRequestError
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from litellm.llms.anthropic.common_utils import AnthropicError
from pydantic import JsonValue as Json, TypeAdapter
from typing_extensions import override

REVERSE_MAP_KEY: Final = "_anthropic_tool_name_map"
Transform = Callable[
    [str, list[Json], dict[str, Json], dict[str, Json], dict[str, Json]],
    Coroutine[Json, Json, dict[str, Json]],
]
# LiteLLM types the messages as AllMessageValues, whose Any members mypy rejects; this validates
# the bound method into the JSON-facing signature the helper calls it with
TRANSFORM = TypeAdapter(Transform)


class Fixture(TypedDict):
    model_cost: dict[str, Json]
    packaged: dict[str, Json]


class Input(TypedDict):
    fixture: Fixture
    model: str
    custom_llm_provider: str | None
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    headers: dict[str, str]
    strips_billing_metadata: bool
    modify_params: bool
    global_drop_params: bool


class CaseConfig(AnthropicConfig):
    provider: str
    strips_billing: bool

    def __init__(self, *, provider: str, strips_billing: bool) -> None:
        self.provider = provider
        self.strips_billing = strips_billing

    @property
    @override
    def custom_llm_provider(self) -> str:
        return self.provider

    @override
    def should_strip_billing_metadata(self) -> bool:
        return self.strips_billing


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def first(self) -> str | None:
        if not self.messages:
            return None
        return self.messages[0]


def answer(input_value: Input) -> Json:
    provider = input_value["custom_llm_provider"]
    if provider is None:
        provider = "anthropic"
    config = CaseConfig(provider=provider, strips_billing=input_value["strips_billing_metadata"])
    transform = TRANSFORM.validate_python(config.async_transform_request)
    messages: list[Json] = list(input_value["messages"])
    optional_params: dict[str, Json] = dict(input_value["optional_params"])
    litellm_params: dict[str, Json] = dict(input_value["litellm_params"])
    headers: dict[str, Json] = dict(input_value["headers"])
    warnings = Warnings()
    logging.getLogger("LiteLLM").addHandler(warnings)
    try:
        body = asyncio.run(
            transform(
                input_value["model"],
                messages,
                optional_params,
                litellm_params,
                headers,
            )
        )
    except BadRequestError as error:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
        return {"raised": True, "status": error.status_code}
    except AnthropicError as error:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L309
        return {"raised": True, "status": error.status_code}
    reverse: Json = None
    if REVERSE_MAP_KEY in litellm_params:
        reverse = litellm_params[REVERSE_MAP_KEY]
    return {"body": body, "headers": headers, "reverse": reverse, "warning": warnings.first()}


def run(input_value: Input) -> Json:
    litellm.model_cost = input_value["fixture"]["model_cost"]
    # the proxy and Router set this, else an unknown model prints a banner to stdout
    litellm.suppress_debug_info = True
    litellm.drop_params = input_value["global_drop_params"]
    litellm.modify_params = input_value["modify_params"]
    return answer(input_value)
