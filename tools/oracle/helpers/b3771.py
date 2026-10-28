from typing import Any, cast

from litellm.proxy.guardrails.guardrail_hooks.grayswan.grayswan import GraySwanGuardrail


def run(input_value: dict[str, Any]) -> Any:
    data = cast(dict[str, Any], input_value)
    if "metadata" in data or "litellm_metadata" in data:
        merged: dict[str, Any] = {}
        merged.update(cast(dict[str, Any], data.get("metadata", {})))
        merged.update(cast(dict[str, Any], data.get("litellm_metadata", {})))
        if "user_api_key_token" in merged:
            merged["user_api_key_hash"] = merged["user_api_key_token"]
            del merged["user_api_key_token"]
        return {key: value for key, value in merged.items() if key.startswith("user_api_key_")}
    if "score" in data or "action" in data:
        action = cast(str, data.get("action", "BLOCK"))
        score = float(data.get("score", 0.0))
        guardrail = GraySwanGuardrail(api_key="oracle-key", on_flagged_action=action)
        try:
            guardrail._process_response({"violation_score": score}, "PROMPT")  # pyright: ignore[reportPrivateUsage]
            return {"allowed": True, "flagged": score >= guardrail.violation_threshold}
        except Exception as error:
            raise RuntimeError(str(error)) from error
    raise ValueError("unsupported operation")
