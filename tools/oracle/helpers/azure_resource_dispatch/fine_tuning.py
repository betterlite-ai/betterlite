import litellm
from pydantic import JsonValue

from azure_fine_tuning import run as fine_tuning_run

from .builders import use_fine_tuning_builder
from .contracts import (
    FIELDS,
    FINE_TUNING_IN,
    STRING,
    VALUE,
    first_step,
    page_value,
    paged,
    reply_payload,
    returned_of,
)

litellm.suppress_debug_info = True


def job_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    job = returned_of(row)
    if kind == "list":
        return page_value(job)
    if kind == "create":
        data = FIELDS.validate_python(row["data"])
        extra = FIELDS.validate_python(data["extra_body"])
        return {
            "id": job["id"],
            "status": job["status"],
            "training_type": extra["trainingType"],
        }
    return {"id": job["id"], "status": job["status"]}


def job_step(kind: str, step: dict[str, JsonValue]) -> dict[str, JsonValue]:
    body = VALUE.validate_json(STRING.validate_python(step["body"]))
    reply: dict[str, JsonValue] = {"status": step["status"], "body": body}
    payload = reply_payload(kind, step, reply)
    if kind == "create":
        payload["data"] = {"model": "gpt-4o-mini", "training_file": "file_a"}
    elif kind == "list":
        paged(payload, step)
    else:
        payload["job_id"] = STRING.validate_python(step["job_id"])
    return payload


def job_answer(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    use_fine_tuning_builder()
    argument: dict[str, JsonValue] = {"steps": [job_step(kind, step)]}
    answer = fine_tuning_run(FINE_TUNING_IN.validate_python(argument))
    return job_value(kind, first_step(answer, "steps"))
