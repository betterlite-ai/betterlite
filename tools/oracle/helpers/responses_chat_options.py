import json
from typing import TypeAlias, TypedDict, cast

import litellm
from litellm.responses.litellm_completion_transformation import transformation
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = JsonValue


class CacheEntry(TypedDict):
    key: str
    value: Json


class Input(TypedDict):
    request: dict[str, Json]
    provider: str | None
    stream: bool | None
    extra_headers: dict[str, Json] | None
    metadata: dict[str, Json] | None
    service_tier: str | None
    api_base: str | None
    supported_openai_params: list[str] | None
    bridge_mode: str | None
    bridge_failure: str | None
    route_all_openai_to_responses: bool
    cache: list[CacheEntry]


class ParameterCall(TypedDict):
    model: str
    custom_llm_provider: str | None


class ProbeCall(TypedDict):
    model: str
    custom_llm_provider: str
    web_search_options: Json
    tools: Json
    reasoning_effort: Json
    reasoning_summary: Json
    api_base: str | None


class Parameters:
    def __init__(self, supported: list[str] | None) -> None:
        self.supported = supported
        self.calls: list[ParameterCall] = []

    def __call__(
        self,
        *,
        model: str,
        custom_llm_provider: str | None = None,
    ) -> list[str] | None:
        self.calls.append({"model": model, "custom_llm_provider": custom_llm_provider})
        return self.supported


class Bridge:
    def __init__(self, mode: str | None, failure: str | None) -> None:
        self.mode = mode
        self.failure = failure
        self.calls: list[ProbeCall] = []

    def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1083
        self,
        *,
        model: str,
        custom_llm_provider: str,
        web_search_options: Json,
        tools: Json,
        reasoning_effort: Json,
        reasoning_summary: Json,
        api_base: str | None,
    ) -> tuple[dict[str, Json], str]:
        self.calls.append(
            {
                "model": model,
                "custom_llm_provider": custom_llm_provider,
                "web_search_options": web_search_options,
                "tools": tools,
                "reasoning_effort": reasoning_effort,
                "reasoning_summary": reasoning_summary,
                "api_base": api_base,
            }
        )
        if self.failure is not None:
            raise RuntimeError(self.failure)
        mode: dict[str, Json] = {} if self.mode is None else {"mode": self.mode}
        return mode, model


class Sink:
    stream_options: Json = None


def run(input_value: Input) -> Json:
    parameters = Parameters(input_value["supported_openai_params"])
    bridge = Bridge(input_value["bridge_mode"], input_value["bridge_failure"])
    transformation.get_supported_openai_params = parameters  # type: ignore[assignment, attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L52
    litellm.main.responses_api_bridge_check = bridge  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1083
    litellm.route_all_chat_openai_to_responses = input_value[
        "route_all_openai_to_responses"
    ]
    sink = Sink()
    request = input_value["request"]
    model = cast("str", request["model"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L402
    supported = cast("Json", ResponsesConfig.get_supported_openai_params(model))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L205
    completion = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L402
        "dict[str, Json]",
        ResponsesConfig.transform_responses_api_request_to_chat_completion_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L402
            model=model,
            input=cast("str | ResponseInputParam", request["input"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L402
            responses_api_request=cast("ResponsesAPIOptionalRequestParams", request),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L402
            custom_llm_provider=input_value["provider"],
            stream=input_value["stream"],
            extra_headers=input_value["extra_headers"],
            metadata=input_value["metadata"],
            service_tier=input_value["service_tier"],
            api_base=input_value["api_base"],
            litellm_logging_obj=sink,
        ),
    )
    wire = TypeAdapter(JsonValue)
    output: dict[str, Json] = {
        "request": completion,
        "supported_options": supported,
        "parameters": wire.validate_python(parameters.calls),
        "probe": wire.validate_python(bridge.calls),
        "logged": sink.stream_options,
    }
    return wire.validate_json(json.dumps(output))
