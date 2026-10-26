from typing import TypeAlias

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def integer(value: Json) -> int:
    return value if isinstance(value, int) and value >= 0 else 0


def cached(usage: dict[str, Json]) -> int:
    details = usage.get("prompt_tokens_details") or usage.get("input_tokens_details")
    if isinstance(details, dict):
        return integer(details.get("cached_tokens"))
    return integer(usage.get("cached_tokens")) or integer(usage.get("cache_read_input_tokens"))


def usage_values(input_value: dict[str, Json]) -> tuple[int, int, int, int]:
    usage = input_value.get("usage")
    if input_value.get("usage_kind") == "response_api":
        usage = input_value.get("response_api_usage")
    if not isinstance(usage, dict):
        return 0, 0, 0, 0
    response_api = input_value.get("usage_kind") == "response_api"
    input_tokens = integer(usage.get("input_tokens" if response_api else "prompt_tokens"))
    output_tokens = integer(usage.get("output_tokens" if response_api else "completion_tokens"))
    total_tokens = integer(usage.get("total_tokens"))
    return input_tokens, output_tokens, total_tokens, cached(usage)


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    configured = input_value.get("rate_limit_type")
    rate_type = configured if configured in ("input", "output", "total") else "total"
    input_tokens, output_tokens, total_tokens, cache = usage_values(input_value)
    rerank_input = integer(input_value.get("rerank_tokens", {}).get("input_tokens")) if isinstance(input_value.get("rerank_tokens"), dict) else 0
    rerank_output = integer(input_value.get("rerank_tokens", {}).get("output_tokens")) if isinstance(input_value.get("rerank_tokens"), dict) else 0
    billed = integer(input_value.get("billed_units_total_tokens"))
    if rerank_input or rerank_output:
        input_tokens, output_tokens = rerank_input, rerank_output
        selected = output_tokens if rate_type == "output" else input_tokens if rate_type == "input" else input_tokens + output_tokens
        resolved = True
    elif billed:
        input_tokens, output_tokens, selected, resolved = billed, 0, billed if rate_type != "output" else 0, True
    else:
        selected = output_tokens if rate_type == "output" else max(0, input_tokens - cache) if rate_type == "input" else max(0, total_tokens - cache)
        input_tokens = max(0, input_tokens - cache)
        resolved = bool(input_tokens or output_tokens)
    aggregate = total_tokens if not input_tokens and not output_tokens else 0
    recovered = [0, 0, 0]
    combined = input_value.get("combined_usage_object")
    if isinstance(combined, dict) and integer(combined.get("completion_tokens")) + integer(combined.get("output_tokens")) > 0:
        combined_input = integer(combined.get("prompt_tokens")) or integer(combined.get("input_tokens"))
        combined_output = integer(combined.get("completion_tokens")) or integer(combined.get("output_tokens"))
        combined_cache = cached(combined)
        recovered = [max(0, integer(combined.get("total_tokens")) - combined_cache) if rate_type == "total" else combined_output if rate_type == "output" else max(0, combined_input - combined_cache), max(0, combined_input - combined_cache), combined_output]
    return {"rate_limit_type": rate_type, "total_tokens": selected, "input_tokens": input_tokens, "output_tokens": output_tokens, "usage_resolved": resolved, "aggregate_only_total_tokens": aggregate, "recovered": recovered}
