from datetime import UTC, datetime
from typing import Literal, Required, TypedDict

import litellm
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.agentic_followup_kwargs import build_agentic_followup_kwargs
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.integrations.custom_logger import AgenticLoopSafetyError
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class CallbackSpec(TypedDict, total=False):
    kind: Required[Literal["override", "plain", "resolved_string", "unresolved_string", "other"]]
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
        super().__init__()
        self.result = result

    @override
    async def async_pre_call_deployment_hook(  # ast-grep-ignore: no-long-names
        self,
        # ast-grep-ignore: no-loose-types
        _kwargs: dict[str, object],
        _call_type: CallTypes | None,
    ) -> dict[str, JsonValue] | None:
        return self.result


def resolve_compatible(name: str) -> CustomLogger | None:
    if name == "datadog":
        return ScriptedCallback(None)
    return None


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
        return {
            "model": model_out,
            "input": JSON.validate_python(input_out),
            "custom_llm_provider": provider_out,
            "optional_params": {
                key: JSON.validate_python(value) for key, value in optional_out.items()
            },
            "generic_params": {
                "api_key": JSON.validate_python(params_out.api_key),
                "api_base": JSON.validate_python(params_out.api_base),
            },
        }

    def loop_settings(
        self,
        # ast-grep-ignore: no-loose-types
        kwargs: dict[str, object],
    ) -> tuple[int, int, list[str]]:
        return self._get_agentic_loop_settings(kwargs)

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


# ast-grep-ignore: no-loose-types
def object_map(values: dict[str, JsonValue]) -> dict[str, object]:
    # ast-grep-ignore: no-loose-types
    params: dict[str, object] = {}
    for key, value in values.items():
        params[key] = value
    return params


def callback_of(spec: CallbackSpec) -> CustomLogger:
    kind = spec["kind"]
    if kind == "override":
        return ScriptedCallback(spec.get("result"))
    if kind == "plain":
        return CustomLogger()
    raise ValueError(f"unsupported dynamic callback kind {kind}")


def install_global(specs: list[CallbackSpec]) -> None:
    for spec in specs:
        kind = spec["kind"]
        if kind == "override":
            litellm.callbacks.append(ScriptedCallback(spec.get("result")))
        elif kind == "plain":
            litellm.callbacks.append(CustomLogger())
        elif kind == "resolved_string":
            litellm.callbacks.append("datadog")
        elif kind == "unresolved_string":
            litellm.callbacks.append("langfuse")
        else:
            litellm.callbacks.append(CustomLogger)


def install_dynamic(logging_obj: Logging, specs: list[CallbackSpec] | None) -> None:
    if specs is None:
        logging_obj.dynamic_success_callbacks = None
        return
    if not specs:
        logging_obj.dynamic_success_callbacks = []
        return
    first = callback_of(specs[0])
    if len(specs) > 1:
        logging_obj.dynamic_success_callbacks = [first, callback_of(specs[1])]
        return
    logging_obj.dynamic_success_callbacks = [first]


def deployment_answer(case: DeploymentCase) -> JsonValue:
    saved = list(litellm.callbacks)
    saved_resolver = litellm_logging.get_custom_logger_compatible_class
    litellm.callbacks.clear()
    litellm_logging.get_custom_logger_compatible_class = resolve_compatible
    try:
        install_global(case["global_callbacks"])
        logging_obj = Logging(
            model=case["model"],
            messages=None,
            stream=False,
            call_type="responses",
            start_time=datetime(2026, 1, 1, tzinfo=UTC),
            litellm_call_id="call-1",
            function_id="fn-1",
        )
        install_dynamic(logging_obj, case["dynamic_callbacks"])
        litellm_params = GenericLiteLLMParams(
            api_key=case["generic_params"].get("api_key"),
            api_base=case["generic_params"].get("api_base"),
        )
        return HookHandler().deployment_outcome(
            case,
            object_map(case["optional_params"]),
            litellm_params,
            logging_obj,
        )
    finally:
        litellm.callbacks.clear()
        litellm.callbacks.extend(saved)
        litellm_logging.get_custom_logger_compatible_class = saved_resolver


def settings_answer(case: SettingsCase) -> JsonValue:
    handler = HookHandler()
    answers = [settings_probe(handler, probe) for probe in case["probes"]]
    return {"probes": answers}


def settings_probe(handler: HookHandler, probe: dict[str, JsonValue]) -> JsonValue:
    try:
        depth, max_loops, fingerprints = handler.loop_settings(object_map(probe))
    except (TypeError, ValueError) as error:
        return {"error": {"class": type(error).__name__, "message": str(error)}}
    return {
        "settings": {
            "depth": depth,
            "max_loops": max_loops,
            "fingerprints": JSON.validate_python(fingerprints),
        }
    }


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
    return JSON.validate_python(dict(result))


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "deployment_hook":
        return deployment_answer(input_value)
    if input_value["operation"] == "settings":
        return settings_answer(input_value)
    if input_value["operation"] == "safety":
        return safety_answer(input_value)
    return followup_answer(input_value)
