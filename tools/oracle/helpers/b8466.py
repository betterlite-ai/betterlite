import asyncio
from typing import Literal, Required, TypedDict

import litellm
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
ARRAY: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])
ROWS: TypeAdapter[list[dict[str, JsonValue]]] = TypeAdapter(list[dict[str, JsonValue]])
STRING: TypeAdapter[str] = TypeAdapter(str)

DIRECT_CALLS: list[dict[str, JsonValue]] = []


class Input(TypedDict, total=False):
    check: Required[Literal["is_prompt_model", "prompt_factory"]]
    model: str
    model_list: list[dict[str, JsonValue]]
    request: dict[str, JsonValue]
    compiled: dict[str, JsonValue]


class _Logging(dict[str, JsonValue]):
    def __init__(self, compiled: dict[str, JsonValue]) -> None:
        super().__init__()
        self.compiled = compiled
        self.compile_call: dict[str, JsonValue] = {}

    def get_chat_completion_prompt(
        self, **kwargs: JsonValue
    ) -> tuple[str, list[JsonValue], dict[str, JsonValue]]:
        self.compile_call = kwargs
        return (
            STRING.validate_python(self.compiled.get("model")),
            ARRAY.validate_python(self.compiled.get("messages")),
            OBJECT.validate_python(self.compiled.get("optional_params")),
        )


class _PromptRouter(Router):
    rows: list[dict[str, JsonValue]]
    captured: dict[str, JsonValue]

    def is_prompt_model(self, model: str) -> bool:
        return self._is_prompt_management_model(model)

    @override
    async def async_get_available_deployment(
        self,
        model: str,
        request_kwargs: dict[str, JsonValue],
        messages: list[dict[str, str]] | None = None,
        input: str | list[JsonValue] | None = None,
        specific_deployment: bool | None = False,
    ) -> dict[str, JsonValue]:
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

    async def run_factory(self, model: str, request: dict[str, JsonValue]) -> str:
        outcome: str = await self._prompt_management_factory(model, [], request)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L4530-L4535
        return outcome


async def record_direct_call(**kwargs: JsonValue) -> JsonValue:
    DIRECT_CALLS.append(dict(kwargs))
    return "direct"


def _factory_output(input_value: Input) -> dict[str, JsonValue]:
    DIRECT_CALLS.clear()
    litellm.acompletion = record_direct_call
    request = OBJECT.validate_python(input_value.get("request"))
    logging = _Logging(OBJECT.validate_python(input_value.get("compiled")))
    request["litellm_logging_obj"] = logging
    rows = ROWS.validate_python(input_value.get("model_list"))
    router = _PromptRouter(model_list=rows)
    router.rows = rows
    router.captured = {}
    model = STRING.validate_python(input_value.get("model"))
    call = asyncio.run(router.run_factory(model, request))
    captured = DIRECT_CALLS[0] if DIRECT_CALLS else router.captured
    kept = captured.pop("litellm_logging_obj") is logging
    request_kwargs = OBJECT.validate_python(logging.compile_call.get("request_kwargs"))
    request_kwargs.pop("litellm_logging_obj")
    logging.compile_call["request_kwargs"] = request_kwargs
    return {
        "compile_call": logging.compile_call,
        "request": captured,
        "logging_kept": kept,
        "call": call,
    }


def run(input_value: Input) -> dict[str, JsonValue]:
    match input_value["check"]:
        case "is_prompt_model":
            router = _PromptRouter(
                model_list=ROWS.validate_python(input_value.get("model_list"))
            )
            return {
                "is_prompt_model": router.is_prompt_model(
                    STRING.validate_python(input_value.get("model"))
                )
            }
        case "prompt_factory":
            return _factory_output(input_value)
