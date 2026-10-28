from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
VIOLATION_THRESHOLD = 0.5


class GuardrailError(Exception):
    pass


def run(input_value: dict[str, Json]) -> Json:
    data = input_value
    if "metadata" in data or "litellm_metadata" in data:
        merged: dict[str, Json] = {}
        merged.update(cast(dict[str, Json], data.get("metadata", {})))
        merged.update(cast(dict[str, Json], data.get("litellm_metadata", {})))
        if "user_api_key_token" in merged:
            merged["user_api_key_hash"] = merged["user_api_key_token"]
            del merged["user_api_key_token"]
        return {key: value for key, value in merged.items() if key.startswith("user_api_key_")}
    if "score" in data or "action" in data:
        action = cast(str, data.get("action", "BLOCK"))
        score = float(cast(float, data.get("score", 0.0)))
        flagged = score >= VIOLATION_THRESHOLD
        if not flagged or action == "MONITOR":
            return {"allowed": True, "flagged": flagged}
        raise GuardrailError("content flagged")
    raise ValueError("unsupported operation")
