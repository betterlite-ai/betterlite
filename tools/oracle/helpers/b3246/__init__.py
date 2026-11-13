import asyncio
from typing import ClassVar, Literal, NotRequired, Protocol, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.guardrail_translation.base_translation import (
    BaseTranslation,
    StreamTransformSink,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.policy_engine.pipeline_executor import PipelineExecutor
from litellm.types.proxy.policy_engine.pipeline_types import PipelineStep
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override


class Assembled(TypedDict):
    texts: list[str]
    tool_calls: list[Json]


class HookScript(TypedDict):
    mode: Literal["nothing", "replace", "mutate"]
    response: Assembled | None


class GuardrailScript(TypedDict):
    kind: Literal["unified", "legacy"]
    name: str
    texts_out: list[str] | None
    tool_calls_out: list[Json] | None
    hook: HookScript


class TranslationScript(TypedDict):
    delivers: bool
    tool_calls: list[Json] | None
    rescans: bool


class Input(TypedDict):
    guardrail: GuardrailScript
    translation: TranslationScript
    chunks: list[str]


class Scanned(TypedDict, total=False):
    texts: list[str]
    tool_calls: list[Json]


class Bucket(TypedDict, total=False):
    applied_guardrails: list[str]


class Observed(TypedDict):
    chunks: list[str]
    metadata: NotRequired[Bucket]
    delivered_tool_calls: NotRequired[list[Json]]


class Output(TypedDict):
    outcome: Literal["pass"]
    chunks: list[str]
    applied_guardrails: list[str]
    delivered_tool_calls: list[Json] | None


class Scripted(Protocol):
    script: GuardrailScript


SCANNED = TypeAdapter(Scanned)
OBSERVED = TypeAdapter(Observed)
ASSEMBLED = TypeAdapter(Assembled)
CHUNKS = TypeAdapter(list[object])
OBJECTS = TypeAdapter(dict[str, object])


async def scan(
    guardrail: CustomGuardrail, inputs: Scanned, data: dict[str, Json], log: Logging | None
) -> Scanned:
    return SCANNED.validate_python(await guardrail.apply_guardrail(inputs, data, "response", log))  # type: ignore[arg-type,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1312


async def scripted_apply(self: Scripted, inputs: Scanned, **_call: Json) -> Scanned:
    outputs: Scanned = {**inputs}
    texts_out = self.script["texts_out"]
    if texts_out is not None:
        outputs["texts"] = texts_out
    tool_calls_out = self.script["tool_calls_out"]
    if tool_calls_out is not None:
        outputs["tool_calls"] = tool_calls_out
    return outputs


async def scripted_hook(self: Scripted, response: Assembled, **_call: Json) -> Assembled | None:
    hook = self.script["hook"]
    scripted = hook["response"]
    if hook["mode"] == "replace":
        return scripted
    if hook["mode"] == "mutate" and scripted is not None:
        response.update(scripted)
    return None


def scripted_guardrail(script: GuardrailScript) -> CustomGuardrail:
    members = OBJECTS.validate_json("{}")
    members["script"] = script
    if script["kind"] == "unified":
        members["apply_guardrail"] = scripted_apply
    else:
        members["async_post_call_success_hook"] = scripted_hook
    return type("Scripted", (CustomGuardrail,), members)(guardrail_name=script["name"])  # type: ignore[no-any-return,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L180


class ScriptedTranslation(BaseTranslation):
    def __init__(self, script: TranslationScript) -> None:
        self.script = script

    @override
    async def process_input_messages(
        self,
        data: dict[str, Json],
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
    ) -> None:
        return None

    @override
    async def process_output_response(
        self,
        response: Json,
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: UserAPIKeyAuth | None = None,
        request_data: dict[str, Json] | None = None,
    ) -> None:
        if not self.script["rescans"]:
            return
        assembled = ASSEMBLED.validate_python(response)
        inputs: Scanned = {"texts": assembled["texts"], "tool_calls": assembled["tool_calls"]}
        await scan(guardrail_to_apply, inputs, {}, litellm_logging_obj)

    @override
    async def process_output_streaming_response(  # ast-grep-ignore: no-long-names
        self,
        responses_so_far: list[str],
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: UserAPIKeyAuth | None = None,
        request_data: dict[str, Json] | None = None,
        stream_transform_sink: StreamTransformSink | None = None,
        deliver_ended_stream_rewrites: bool = False,
    ) -> None:
        if request_data is None:
            raise ValueError("a streaming step always passes request_data")
        texts = [*responses_so_far]
        tool_calls = self.script["tool_calls"]
        stored_calls = [] if tool_calls is None else tool_calls
        request_data["response"] = {"texts": [*texts], "tool_calls": [*stored_calls]}
        inputs: Scanned = {"texts": texts}
        if tool_calls is not None:
            inputs["tool_calls"] = tool_calls
        outputs = await scan(guardrail_to_apply, inputs, request_data, litellm_logging_obj)
        rewritten = outputs.get("texts")
        if (
            deliver_ended_stream_rewrites
            and rewritten is not None
            and len(rewritten) == len(responses_so_far)
        ):
            responses_so_far[:] = rewritten
        delivered = outputs.get("tool_calls")
        if delivered is not None:
            request_data["delivered_tool_calls"] = delivered


class DeliveringTranslation(ScriptedTranslation):
    delivers_ended_stream_rewrites: ClassVar[bool] = True


def run(input_value: Input) -> Output:
    script = input_value["translation"]
    translation = (
        DeliveringTranslation(script) if script["delivers"] else ScriptedTranslation(script)
    )
    guardrail = scripted_guardrail(input_value["guardrail"])
    chunks = CHUNKS.validate_python(input_value["chunks"])
    hook_input = OBJECTS.validate_json("{}")
    step = PipelineStep(guardrail=input_value["guardrail"]["name"])
    run_step = PipelineExecutor._run_streaming_step  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/policy_engine/pipeline_executor.py#L428
    asyncio.run(run_step(step, guardrail, translation, chunks, hook_input, UserAPIKeyAuth(), None))
    hook_input["chunks"] = chunks
    observed = OBSERVED.validate_python(hook_input)
    bucket = observed.get("metadata")
    applied = [] if bucket is None else bucket.get("applied_guardrails")
    return {
        "outcome": "pass",
        "chunks": observed["chunks"],
        "applied_guardrails": [] if applied is None else applied,
        "delivered_tool_calls": observed.get("delivered_tool_calls"),
    }
