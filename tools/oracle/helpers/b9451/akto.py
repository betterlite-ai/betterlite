import asyncio
from typing import Literal, Protocol, TypedDict

from litellm.proxy.guardrails.guardrail_hooks.akto import initialize_guardrail
from litellm.types.guardrails import LitellmParams
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .scripted import Answer, attempt, outcome_view, scripted

ENCODED_FIELDS = ("requestHeaders", "responseHeaders", "requestPayload", "responsePayload", "tag", "metadata")


class AktoCase(TypedDict):
    vendor: Literal["akto"]
    akto_api_key: str
    unreachable_fallback: Literal["fail_closed", "fail_open"]
    event_hook: str | list[str]
    texts: list[str]
    request_data: dict[str, Json]
    input_type: Literal["request", "response"]
    answers: list[Answer]


# LiteLLM types request_data as a bare dict and keeps its background sends in a bare set
class AktoHooks(Protocol):
    background_tasks: set[asyncio.Task[None]]

    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, Json],
        input_type: Literal["request", "response"],
        logging_obj: None = None,
    ) -> GenericGuardrailAPIInputs: ...


def decoded_body(body: Json) -> Json:
    fields = TypeAdapter(dict[str, Json]).validate_python(body)
    fields["time"] = "<millis>"
    for name in ENCODED_FIELDS:
        inner = TypeAdapter(dict[str, Json]).validate_json(TypeAdapter(str).validate_python(fields[name]))
        if "body" in inner:
            # the payloads are JSON text inside JSON text
            inner["body"] = TypeAdapter(Json).validate_json(TypeAdapter(str).validate_python(inner["body"]))
        fields[name] = inner
    return fields


async def call_hook(guardrail: AktoHooks, case: AktoCase) -> dict[str, Json]:
    inputs: GenericGuardrailAPIInputs = {"texts": case["texts"]}
    returned = await guardrail.apply_guardrail(inputs, case["request_data"], case["input_type"])
    texts = returned.get("texts")
    return {"texts": None if texts is None else [*texts]}


async def run_akto(case: AktoCase) -> Json:
    async with scripted(case["answers"]) as server:
        # the YAML initializer hands the mode over as the plain string HOOK_TO_INPUT compares
        params = LitellmParams(
            guardrail="akto",
            mode=case["event_hook"],
            akto_base_url=server.base_url,
            akto_api_key=case["akto_api_key"],
            unreachable_fallback=case["unreachable_fallback"],
        )
        guardrail: AktoHooks = initialize_guardrail(params, {"guardrail_name": "akto", "litellm_params": params})
        outcome = await attempt(call_hook(guardrail, case))
        await asyncio.gather(*guardrail.background_tasks)
        for call in server.seen:
            call["body"] = decoded_body(call["body"])
        return outcome_view(server, outcome, ("content-type",), {"authorization": case["akto_api_key"]})
