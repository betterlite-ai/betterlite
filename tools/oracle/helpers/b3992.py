from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: Json) -> Json:  # noqa: PLR0911
    request = cast(dict[str, Json], input_value)
    operation = cast(str, request.get("operation"))
    if operation == "precedence":
        audit = cast(dict[str, Json], request.get("audit", {}))
        global_source = cast(dict[str, Json], request.get("global", {}))
        return {"bucket_name": audit.get("s3_bucket_name", global_source.get("s3_bucket_name"))}
    if operation == "defaults":
        return {"max_queue_size": 50000, "max_retry_age_seconds": 3600}
    if operation == "retry":
        source = cast(dict[str, Json], request.get("source", {}))
        retry_age = source.get("s3_max_retry_age_seconds", 3600)
        if not isinstance(retry_age, int) or isinstance(retry_age, bool) or retry_age < 0:
            retry_age = 3600
        return {"max_retry_age_seconds": None if retry_age == 0 else retry_age}
    if operation == "partition":
        values = cast(list[Json], request.get("values", []))
        return [value if value in ("day", "hour") else "day" for value in values]
    if operation == "prompts":
        payload = dict(cast(dict[str, Json], request.get("payload", {})))
        payload["response"] = None
        return payload
    if operation == "strip":
        return _strip(request.get("payload"))
    if operation == "cold":
        return {"accepted": request.get("cold_storage_object_key") == "accepted"}
    if operation == "event":
        payload = request.get("payload")
        return {"payload": payload}
    raise ValueError(f"unknown b3992 operation: {operation}")


def _strip(value: Json) -> Json:
    if isinstance(value, list):
        return [_strip(item) for item in value]
    if isinstance(value, dict):
        retained = {
            key: item
            for key, item in value.items()
            if key != "file" and not (key == "type" and item != "text")
        }
        return {key: _strip(item) for key, item in retained.items()}
    if isinstance(value, str) and value.startswith("data:") and ";base64," in value:
        return "[BASE64_REDACTED]"
    return value
