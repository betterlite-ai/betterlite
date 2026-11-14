import asyncio
from typing import Literal, Required, TypedDict, Unpack

import litellm
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
ARRAY: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])
ROWS: TypeAdapter[list[dict[str, JsonValue]]] = TypeAdapter(list[dict[str, JsonValue]])
STRING: TypeAdapter[str] = TypeAdapter(str)

DIRECT_CALLS: list[dict[str, JsonValue]] = []
PROBE_COMPILED: dict[str, JsonValue] = {
    "model": "probe-model",
    "messages": [],
    "optional_params": {},
}
# Router.acompletion fills these in itself and the trace id is random.
ROUTER_ONLY_KEYS: tuple[str, ...] = ("litellm_trace_id", "original_function")


class Input(TypedDict, total=False):
    check: Required[Literal["is_prompt_model", "prompt_factory"]]
    model: str
    model_list: list[dict[str, JsonValue]]
    request: dict[str, JsonValue]
    messages: list[JsonValue]
    compiled: dict[str, JsonValue]


def _scrub(kwargs: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        key: value
        for key, value in kwargs.items()
        if key not in ROUTER_ONLY_KEYS and key != "litellm_logging_obj"
    }


class CompileCall(TypedDict):
    model: str
    messages: list[JsonValue]
    non_default_params: dict[str, JsonValue]
    prompt_id: str | None
    prompt_variables: dict[str, JsonValue] | None
    prompt_label: str | None
    request_kwargs: dict[str, JsonValue]
    injected_for_every_deployment: bool


class _Logging(dict[str, JsonValue]):
    def __init__(self, compiled: dict[str, JsonValue]) -> None:
        super().__init__()
        self.compiled = compiled
        self.compile_call: dict[str, JsonValue] = {}

    def get_chat_completion_prompt(
        self, **kwargs: Unpack[CompileCall]
    ) -> tuple[str, list[JsonValue], dict[str, JsonValue]]:
        self.compile_call = {
            "model": kwargs["model"],
            "messages": kwargs["messages"],
            "non_default_params": _scrub(kwargs["non_default_params"]),
            "prompt_id": kwargs["prompt_id"],
            "prompt_variables": kwargs["prompt_variables"],
            "prompt_label": kwargs["prompt_label"],
            "request_kwargs": _scrub(kwargs["request_kwargs"]),
            "injected_for_every_deployment": kwargs["injected_for_every_deployment"],
        }
        return (
            STRING.validate_python(self.compiled.get("model")),
            ARRAY.validate_python(self.compiled.get("messages")),
            OBJECT.validate_python(self.compiled.get("optional_params")),
        )


class _PromptRouter(Router):
    rows: list[dict[str, JsonValue]]
    captured: dict[str, JsonValue]
    selection: dict[str, JsonValue]
    factory_reached: bool

    @override
    async def async_get_available_deployment(
        self,
        model: str,
        request_kwargs: dict[str, JsonValue],
        messages: list[dict[str, str]] | None = None,
        input: str | list[JsonValue] | None = None,
        specific_deployment: bool | None = False,
    ) -> dict[str, JsonValue]:
        self.factory_reached = True
        self.selection = {
            "model": model,
            "messages": ARRAY.validate_python(messages),
            "specific_deployment": specific_deployment,
            "request_kwargs": _scrub(request_kwargs),
        }
        return next(row for row in self.rows if row.get("model_name") == model)

    @override
    def _update_kwargs_with_deployment(
        self,
        deployment: dict[str, JsonValue],
        kwargs: dict[str, JsonValue],
        function_name: str | None = None,
    ) -> None:
        return None

    @override
    async def async_function_with_fallbacks(self, **kwargs: JsonValue) -> JsonValue:
        self.captured = dict(kwargs)
        return "with_fallbacks"


async def record_direct_call(**kwargs: JsonValue) -> JsonValue:
    DIRECT_CALLS.append(dict(kwargs))
    return "direct"


def _dispatch(
    input_value: Input, request: dict[str, JsonValue], messages: list[JsonValue]
) -> _PromptRouter:
    DIRECT_CALLS.clear()
    litellm.acompletion = record_direct_call
    rows = ROWS.validate_python(input_value.get("model_list"))
    router = _PromptRouter(model_list=rows)
    router.rows = rows
    router.captured = {}
    router.selection = {}
    router.factory_reached = False
    model = STRING.validate_python(input_value.get("model"))
    asyncio.run(router.acompletion(model, messages, **request))  # type: ignore[call-overload, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L2779-L2802
    return router


def _factory_output(input_value: Input) -> dict[str, JsonValue]:
    request = OBJECT.validate_python(input_value.get("request"))
    logging = _Logging(OBJECT.validate_python(input_value.get("compiled")))
    request["litellm_logging_obj"] = logging
    messages = ARRAY.validate_python(input_value.get("messages"))
    router = _dispatch(input_value, request, messages)
    call = "direct" if DIRECT_CALLS else "with_fallbacks"
    captured = DIRECT_CALLS[0] if DIRECT_CALLS else router.captured
    kept = captured.pop("litellm_logging_obj") is logging
    original_function_kept = "original_function" in captured
    for key in ROUTER_ONLY_KEYS:
        captured.pop(key, None)
    return {
        "compile_call": logging.compile_call,
        "request": captured,
        "logging_kept": kept,
        "selection": router.selection,
        "original_function_kept": original_function_kept,
        "call": call,
    }


def _is_prompt_model_output(input_value: Input) -> dict[str, JsonValue]:
    request: dict[str, JsonValue] = {
        "prompt_id": "probe",
        "litellm_logging_obj": _Logging(PROBE_COMPILED),
    }
    return {"is_prompt_model": _dispatch(input_value, request, []).factory_reached}


def run(input_value: Input) -> dict[str, JsonValue]:
    match input_value["check"]:
        case "is_prompt_model":
            return _is_prompt_model_output(input_value)
        case "prompt_factory":
            return _factory_output(input_value)
