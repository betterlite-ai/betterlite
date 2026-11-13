from collections.abc import Callable, Mapping
from dataclasses import dataclass
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.vertex_ai.interactions.transformation import VertexAIInteractionsConfig
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

litellm.suppress_debug_info = True

HeadersBuilder: TypeAlias = Callable[[Mapping[str, str], str, GenericLiteLLMParams], dict[str, str]]
LifecycleBuilder: TypeAlias = Callable[
    [str, str, GenericLiteLLMParams, Mapping[str, str]], tuple[str, dict[str, JsonValue]]
]
NO_HEADERS: Final[Mapping[str, str]] = {}


class Call(TypedDict):
    function: Literal["url", "lifecycle", "headers"]
    api_base: str
    stream: bool
    params: dict[str, str]
    interaction_id: str
    headers: dict[str, str]
    action: NotRequired[Literal["get", "delete", "cancel"]]


class Input(TypedDict):
    mint: dict[str, str]
    calls: list[Call]


class InteractionView(TypedDict):
    headers: HeadersBuilder
    get: LifecycleBuilder
    delete: LifecycleBuilder
    cancel: LifecycleBuilder


@dataclass(frozen=True)
class MintFixture:
    token: str
    project: str | None

    def __call__(
        self, credentials: VERTEX_CREDENTIALS_TYPES | None, project_id: str | None
    ) -> tuple[str, str]:
        del credentials
        resolved = self.project if self.project is not None else project_id
        return (self.token, resolved if resolved is not None else "")


def resolve(config: VertexAIInteractionsConfig, view: InteractionView, call: Call) -> JsonValue:
    params = GenericLiteLLMParams().model_validate(call["params"])
    if call["function"] == "url":
        url = config.get_complete_url(call["api_base"], None, None, call["params"], call["stream"])
        return {"url": url}
    if call["function"] == "headers":
        headers = view["headers"](call["headers"], "interaction", params)
        return {"headers": dict[str, JsonValue](headers)}
    action = call.get("action")
    if action is None:
        raise ValueError("a lifecycle call needs an action")
    url, body = view[action](call["interaction_id"], call["api_base"], params, NO_HEADERS)
    return {"url": url, "body": body}


def run(input_value: Input) -> JsonValue:
    mint = input_value["mint"]
    fixture = MintFixture(token=mint["token"], project=mint.get("project"))
    config = VertexAIInteractionsConfig(fixture)
    view: InteractionView = cast(InteractionView, {"headers": config.validate_environment, "get": config.transform_get_interaction_request, "delete": config.transform_delete_interaction_request, "cancel": config.transform_cancel_interaction_request})  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/interactions/transformation.py#L87-L149
    return {"calls": [resolve(config, view, call) for call in input_value["calls"]]}
