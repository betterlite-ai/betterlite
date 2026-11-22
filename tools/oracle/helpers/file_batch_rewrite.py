from typing import Literal, TypedDict

from fastapi import HTTPException
from litellm.proxy.openai_files_endpoints import batch_guardrails
from litellm.proxy.openai_files_endpoints.batch_guardrails import (
    BatchScanFailure,
    UnparseableRecord,
    UnroutableRecord,
    UnscannableRecord,
)
from pydantic import JsonValue


class Problem(TypedDict):
    line_number: int
    kind: Literal["unparseable", "unscannable", "unroutable", "raised"]
    custom_id: str | None
    url: str | None
    guardrail_name: str | None
    status: int


class Scenario(TypedDict):
    problems: list[Problem]


class Input(TypedDict):
    scenarios: list[Scenario]


def run(input_value: Input) -> JsonValue:
    selected = [chosen(scenario["problems"]) for scenario in input_value["scenarios"]]
    return {"selected": selected}


def chosen(problems: list[Problem]) -> JsonValue:
    entries: list[tuple[int, BatchScanFailure | BaseException]] = []
    descriptions: list[JsonValue] = []
    for item in problems:
        failure = scan_failure(item)
        if failure is None:
            raised: BaseException = HTTPException(status_code=item["status"], detail=None)
            entries.append((item["line_number"], raised))
        else:
            entries.append((item["line_number"], failure))
        descriptions.append(describe(item))
    worst = batch_guardrails._worst(tuple(entries))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L456
    for index, entry in enumerate(entries):
        if entry[1] is worst:
            return descriptions[index]
    raise ValueError("_worst returned a problem it was not given")


def scan_failure(item: Problem) -> BatchScanFailure | None:
    if item["kind"] == "unparseable":
        return UnparseableRecord(item["line_number"])
    if item["kind"] == "unscannable":
        return UnscannableRecord(item["line_number"], item["custom_id"], item["url"])
    if item["kind"] == "unroutable":
        return UnroutableRecord(item["line_number"], item["custom_id"], item["guardrail_name"])
    return None


def describe(item: Problem) -> JsonValue:
    line_number = item["line_number"]
    if item["kind"] == "unparseable":
        return {"line_number": line_number, "kind": "unparseable"}
    if item["kind"] == "unscannable":
        return {
            "line_number": line_number,
            "kind": "unscannable",
            "custom_id": item["custom_id"],
            "url": item["url"],
        }
    if item["kind"] == "unroutable":
        return {
            "line_number": line_number,
            "kind": "unroutable",
            "custom_id": item["custom_id"],
            "guardrail": item["guardrail_name"],
        }
    return {"line_number": line_number, "kind": "raised", "status": item["status"]}
