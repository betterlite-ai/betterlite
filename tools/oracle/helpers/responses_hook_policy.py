from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from typing import Literal, Required, TypedDict

import litellm
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.agentic_followup_kwargs import build_agentic_followup_kwargs
from litellm.litellm_core_utils.agentic_loop_settings import (
    DEFAULT_MAX_AGENTIC_LOOPS,
    validated_max_agentic_loops,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.integrations.custom_logger import AgenticLoopSafetyError
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MERGE: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
FOLLOWUP: TypeAdapter[Mapping[str, JsonValue]] = TypeAdapter(Mapping[str, JsonValue])

CEILING_FIELD = "litellm_params.max_agentic_loops"

CallbackEntry = CustomLogger | Literal["langfuse"] | Callable[[], None]


class CallbackSpec(TypedDict, total=False):
    kind: Required[Literal["override", "plain", "unresolved_string", "other"]]
    result: dict[str, JsonValue]


class DeploymentCase(TypedDict):
    operation: Literal["deployment_hook"]
    model: str
    input: str
    custom_llm_provider: str
    optional_params: dict[str, JsonValue]
    generic_params: dict[str, str]
    global_callbacks: list[CallbackSpec]
    dynamic_callbacks: list[CallbackSpec] | None


class SettingsCase(TypedDict):
    operation: Literal["settings"]
    probes: list[dict[str, JsonValue]]


class SafetyCase(TypedDict):
    operation: Literal["safety"]
    tool_calls: JsonValue
    fingerprints: list[str]
    depth: int
    max_loops: int
    model: str
    blocks: list[JsonValue]
    stream: bool
    api_surface: str


class FollowupCase(TypedDict):
    operation: Literal["followup"]
    request_kwargs: dict[str, JsonValue]
    patch_kwargs: dict[str, JsonValue]
    request_params: list[str]
    depth: int
    max_loops: int
    fingerprints: list[str]
    fingerprint: str


Input = DeploymentCase | SettingsCase | SafetyCase | FollowupCase


class ScriptedCallback(CustomLogger):
    def __init__(self, result: dict[str, JsonValue] | None) -> None:
        self.result = result

    @override
    async def async_pre_call_deployment_hook(
        self,
        # ast-grep-ignore: no-loose-types
        kwargs: dict[str, object],
        call_type: CallTypes | None,
    ) -> dict[str, JsonValue] | None:
        result = self.result
        if call_type is not CallTypes.responses or result is None:
            return None
        merged = MERGE.validate_python(kwargs)
        merged.update(result)
        return merged


class HookHandler(BaseLLMHTTPHandler):
    def deployment_outcome(
        self,
        case: DeploymentCase,
        # ast-grep-ignore: no-loose-types
        optional_params: dict[str, object],
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
    ) -> JsonValue:
        outcome = self._run_sync_responses_pre_call_deployment_hook(
            model=case["model"],
            input=case["input"],
            custom_llm_provider=case["custom_llm_provider"],
            response_api_optional_request_params=optional_params,
            litellm_params=litellm_params,
            logging_obj=logging_obj,
        )
        model_out, input_out, provider_out, optional_out, params_out = outcome
        answer: dict[str, JsonValue] = {}
        answer["model"] = model_out
        answer["input"] = JSON.validate_python(input_out)
        answer["custom_llm_provider"] = provider_out
        answer["optional_params"] = {
            key: JSON.validate_python(value) for key, value in optional_out.items()
        }
        answer["generic_params"] = {
            "api_key": JSON.validate_python(params_out.api_key),
            "api_base": JSON.validate_python(params_out.api_base),
        }
        return answer

    def tool_fingerprint(self, tool_calls: JsonValue) -> str:
        return self._fingerprint_agentic_tools(tool_calls)

    def safety(self, case: SafetyCase) -> str:
        return self._check_agentic_loop_safety(
            case["tool_calls"],
            case["fingerprints"],
            case["depth"],
            case["max_loops"],
            case["model"],
        )

    def refused(self, tool_calls: JsonValue) -> tuple[frozenset[str], frozenset[str]]:
        return self._refused_agentic_tool_identifiers(tool_calls)

    def refused_block(
        self, block: JsonValue, refused_ids: frozenset[str], refused_names: frozenset[str]
    ) -> bool:
        return self._is_refused_tool_use_block(block, refused_ids, refused_names)

    def terminal(self, *, stream: bool, api_surface: str) -> bool:
        return self._can_replace_turn_with_terminal_response(stream, api_surface)


# stands in for a callback that is neither a logger nor a resolvable name
def other_probe() -> None: ...


def callbacks_from(specs: list[CallbackSpec]) -> list[CallbackEntry]:
    callbacks: list[CallbackEntry] = []
    for spec in specs:
        kind = spec["kind"]
        if kind == "override":
            callbacks.append(ScriptedCallback(spec.get("result")))
        elif kind == "plain":
            callbacks.append(CustomLogger())
        elif kind == "unresolved_string":
            callbacks.append("langfuse")
        else:
            callbacks.append(other_probe)
    return callbacks


def deployment_answer(case: DeploymentCase) -> JsonValue:
    litellm.callbacks = [*callbacks_from(case["global_callbacks"])]
    logging_obj = Logging(
        model=case["model"],
        messages=None,
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    dynamic = case["dynamic_callbacks"]
    if dynamic is not None:
        logging_obj.dynamic_success_callbacks = [*callbacks_from(dynamic)]
    litellm_params = GenericLiteLLMParams(
        api_key=case["generic_params"].get("api_key"),
        api_base=case["generic_params"].get("api_base"),
    )
    return HookHandler().deployment_outcome(
        case,
        {**case["optional_params"]},
        litellm_params,
        logging_obj,
    )


def settings_answer(case: SettingsCase) -> JsonValue:
    answers: list[JsonValue] = [settings_probe(probe) for probe in case["probes"]]
    return {"probes": answers}


def settings_probe(probe: dict[str, JsonValue]) -> JsonValue:
    try:
        ceiling = validated_max_agentic_loops(probe.get("max_agentic_loops"), CEILING_FIELD)
    except (TypeError, ValueError) as error:
        return {"error": {"class": type(error).__name__, "message": str(error)}}
    return {"max_loops": DEFAULT_MAX_AGENTIC_LOOPS if ceiling is None else ceiling}


def safety_answer(case: SafetyCase) -> JsonValue:
    handler = HookHandler()
    refused_ids, refused_names = handler.refused(case["tool_calls"])
    blocks: list[JsonValue] = [
        JSON.validate_python(handler.refused_block(block, refused_ids, refused_names))
        for block in case["blocks"]
    ]
    try:
        safety: JsonValue = {"fingerprint": handler.safety(case)}
    except AgenticLoopSafetyError as error:
        safety = {"error": {"class": type(error).__name__, "message": str(error)}}
    return {
        "fingerprint": handler.tool_fingerprint(case["tool_calls"]),
        "safety": safety,
        "refused": {
            "ids": [JSON.validate_python(value) for value in sorted(refused_ids)],
            "names": [JSON.validate_python(value) for value in sorted(refused_names)],
        },
        "blocks": blocks,
        "terminal": handler.terminal(stream=case["stream"], api_surface=case["api_surface"]),
    }


def followup_answer(case: FollowupCase) -> JsonValue:
    result = build_agentic_followup_kwargs(
        request_kwargs=case["request_kwargs"],
        patch_kwargs=case["patch_kwargs"],
        request_params=case["request_params"],
        depth=case["depth"],
        max_loops=case["max_loops"],
        fingerprints=case["fingerprints"],
        fingerprint=case["fingerprint"],
    )
    return JSON.validate_python(FOLLOWUP.validate_python(result))


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "deployment_hook":
        return deployment_answer(input_value)
    if input_value["operation"] == "settings":
        return settings_answer(input_value)
    if input_value["operation"] == "safety":
        return safety_answer(input_value)
    return followup_answer(input_value)
