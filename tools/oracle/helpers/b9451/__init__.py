import asyncio
from typing import Literal, TypedDict

from litellm.proxy.guardrails.guardrail_hooks.conduct.conduct import ConductGuardrail
from pydantic import JsonValue as Json

from .aim import AimCase, run_aim
from .akto import AktoCase, run_akto
from .lakera import LakeraCase, run_lakera
from .lasso import LassoCase, run_lasso
from .scripted import Answer, attempt, outcome_view, scripted


class ConductCase(TypedDict):
    vendor: Literal["conduct"]
    name: str
    answers: list[Answer]


async def build_conduct(case: ConductCase) -> dict[str, Json]:
    ConductGuardrail(guardrail_name=case["name"])
    return {"built": True}


async def run_conduct(case: ConductCase) -> Json:
    async with scripted(case["answers"]) as server:
        return outcome_view(server, await attempt(build_conduct(case)), (), {})


def run(input_value: AimCase | AktoCase | ConductCase | LakeraCase | LassoCase) -> Json:
    if input_value["vendor"] == "aim":
        return asyncio.run(run_aim(input_value))
    if input_value["vendor"] == "akto":
        return asyncio.run(run_akto(input_value))
    if input_value["vendor"] == "conduct":
        return asyncio.run(run_conduct(input_value))
    if input_value["vendor"] == "lakera":
        return asyncio.run(run_lakera(input_value))
    return asyncio.run(run_lasso(input_value))
