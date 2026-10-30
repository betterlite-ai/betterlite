from typing import cast

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    response = input_value.get("response")
    if function == "unwrap_verdict":
        result = unwrap(response)
    elif function == "has_decision_fields":
        result = has_decision(response)
    elif function == "normalize_action":
        result = normalize(cast(str, input_value.get("action")))
    elif function == "extract_jsonrpc_error":
        result = extract_error(response)
    elif function == "extract_sanitized_text":
        result = extract_text(response)
    elif function == "extract_sanitized_messages":
        result = extract_messages(response)
    elif function == "extract_sanitized_mcp_arguments":
        result = extract_arguments(response)
    elif function == "extract_masked_entity_count":
        result = count_entities(input_value.get("rules"))
    else:
        raise ValueError(f"unknown function: {function}")
    return result


FIELDS = {
    "action",
    "allowed",
    "blocked",
    "safe",
    "is_safe",
    "decision",
    "verdict",
    "status",
    "score",
    "risk_score",
    "confidence",
    "categories",
    "classifications",
    "violations",
    "threats",
    "policies",
    "reason",
    "rules",
    "sanitized_text",
    "sanitizedText",
    "sanitized_payload",
}


def has_decision(value: Json) -> bool:
    return isinstance(value, dict) and bool(FIELDS.intersection(value))


def unwrap(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        return {}
    if has_decision(value):
        return value
    for key in ("result", "data", "inspection", "ai_defense", "aiDefense"):
        nested = value.get(key)
        if has_decision(nested):
            return cast(dict[str, Json], nested)
    result = value.get("result")
    if isinstance(result, dict):
        for key in ("data", "inspection", "ai_defense", "aiDefense"):
            nested = result.get(key)
            if has_decision(nested):
                return cast(dict[str, Json], nested)
    return value


def normalize(value: str) -> str:
    normalized = value.strip().lower()
    if normalized in {
        "deny",
        "denied",
        "block",
        "blocked",
        "reject",
        "rejected",
        "unsafe",
        "malicious",
    }:
        return "block"
    if normalized in {"redact", "redacted", "sanitize", "sanitized", "mask"}:
        return "redact"
    if normalized in {"allow", "allowed", "safe", "ok"}:
        return "allow"
    return "block"


def extract_error(value: Json) -> Json:
    if not isinstance(value, dict):
        return None
    error = value.get("error")
    if isinstance(error, dict):
        return error
    result = value.get("result")
    if isinstance(result, dict) and isinstance(result.get("error"), dict):
        return result["error"]
    return None


def containers(value: Json) -> list[dict[str, Json]]:
    if not isinstance(value, dict):
        return []
    output = [value]
    for key in ("result", "data"):
        nested = value.get(key)
        if isinstance(nested, dict):
            output.append(nested)
    return output


def extract_text(value: Json) -> Json:
    for container in containers(value):
        for key in ("sanitized_text", "sanitizedText"):
            if isinstance(container.get(key), str) and container[key]:
                return container[key]
    return None


def extract_messages(value: Json) -> Json:
    for container in containers(value):
        for key in (
            "sanitized_messages",
            "sanitizedMessages",
            "modified_messages",
            "modifiedMessages",
        ):
            messages = container.get(key)
            if isinstance(messages, list) and messages:
                return [item for item in messages if isinstance(item, dict)]
        for key in ("sanitized_payload", "sanitizedPayload", "modified_payload", "modifiedPayload"):
            payload = container.get(key)
            if (
                isinstance(payload, dict)
                and isinstance(payload.get("messages"), list)
                and payload["messages"]
            ):
                messages = payload["messages"]
                return [item for item in messages if isinstance(item, dict)]  # pyright: ignore[reportOptionalIterable, reportGeneralTypeIssues, reportUnknownVariableType]  # Json unions obscure list narrowing
    return None


def extract_arguments(value: Json) -> Json:
    for container in containers(value):
        params = container.get("params")
        arguments = params.get("arguments") if isinstance(params, dict) else None
        if isinstance(arguments, dict) and arguments:
            return dict(arguments)
        for key in ("sanitized_payload", "sanitizedPayload", "modified_payload", "modifiedPayload"):
            payload = container.get(key)
            if not isinstance(payload, dict):
                continue
            inner = payload.get("params")
            arguments = inner.get("arguments") if isinstance(inner, dict) else None
            if isinstance(arguments, dict) and arguments:
                return dict(arguments)
            arguments = payload.get("arguments")
            if isinstance(arguments, dict) and arguments:
                return dict(arguments)
        arguments = container.get("arguments")
        if isinstance(arguments, dict) and arguments:
            return dict(arguments)
    return None


def count_entities(value: Json) -> Json:
    if not isinstance(value, list):
        return None
    counts: dict[str, int] = {}
    for rule in value:
        if not isinstance(rule, dict) or not isinstance(rule.get("entity_types"), list):
            continue
        entity_types = rule["entity_types"]
        for entity_type in entity_types:  # pyright: ignore[reportOptionalIterable, reportGeneralTypeIssues, reportUnknownVariableType]  # Json unions obscure list narrowing
            if isinstance(entity_type, str):
                counts[entity_type] = counts.get(entity_type, 0) + 1
    return cast(Json, counts or None)
