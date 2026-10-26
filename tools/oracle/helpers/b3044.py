from typing import TypeAlias

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def integer(value: Json) -> int:
    return value if isinstance(value, int) and value >= 0 else 0


def cached(usage: Json) -> int:
    if not isinstance(usage, dict):
        return 0
    details = usage.get("prompt_tokens_details")
    if isinstance(details, dict):
        return integer(details.get("cached_tokens"))
    return integer(usage.get("cache_read_input_tokens"))


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    usage = input_value.get("usage")
    usage_object = usage if isinstance(usage, dict) else {}
    rate_type = input_value.get("rate_limit_type")
    selected_type = rate_type if rate_type in ("input", "output", "total") else "total"
    input_tokens = integer(usage_object.get("prompt_tokens", usage_object.get("input_tokens", 0)))
    output_tokens = integer(usage_object.get("completion_tokens", usage_object.get("output_tokens", 0)))
    total_tokens = max(0, integer(usage_object.get("total_tokens")) - cached(usage))
    selected = {
        "input": max(0, input_tokens - cached(usage)),
        "output": output_tokens,
        "total": total_tokens,
    }[selected_type]
    return {
        "rate_limit_type": selected_type,
        "total_tokens": selected,
        "input_tokens": max(0, input_tokens - cached(usage)),
        "output_tokens": output_tokens,
    }
