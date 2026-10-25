from typing import Literal, TypeAlias, TypedDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
class MessageInput(TypedDict):
    function: Literal["message"]
    message: Json | None
    depth: int | None
    max_depth: int | None
class ResponseInput(TypedDict):
    function: Literal["response"]
    response: dict[str, Json]
    max_depth: int | None
Input: TypeAlias = MessageInput | ResponseInput
def extract_message(message: Json | None, depth: int, max_depth: int) -> str:
    if message is None or depth >= max_depth:
        return ""
    if not isinstance(message, dict):
        raise TypeError("A2A message must be an object")
    parts = message.get("parts", [])
    if not isinstance(parts, list):
        raise TypeError("A2A message parts must be an array")
    text_parts: list[str] = []
    for raw_part in parts:
        if not isinstance(raw_part, dict):
            raise TypeError("A2A message part must be an object")
        if raw_part.get("kind") == "text":
            text = raw_part.get("text", "")
            if not isinstance(text, str):
                raise TypeError("A2A text part text must be a string")
            text_parts.append(text)
        elif "parts" in raw_part:
            nested = extract_message(raw_part, depth + 1, max_depth)
            if nested:
                text_parts.append(nested)
    return " ".join(text_parts)
def truthy(value: Json) -> bool:
    return bool(value)
def extract_response(response: dict[str, Json], max_depth: int) -> str:
    result = response.get("result", {})
    if not isinstance(result, dict):
        selected: Json = None
    elif "parts" in result:
        selected = result
    else:
        message = result.get("message")
        artifact = result.get("artifact")
        status = result.get("status", {})
        status_message = status.get("message") if isinstance(status, dict) else None
        artifacts = result.get("artifacts", [])
        if truthy(message):
            selected = message
        elif truthy(artifact) and isinstance(artifact, dict):
            selected = artifact
        elif truthy(status_message):
            selected = status_message
        elif truthy(artifacts):
            if not isinstance(artifacts, list):
                raise TypeError("A2A artifacts must be an array")
            selected = artifacts[0]
        else:
            selected = None
    return "" if selected is None else extract_message(selected, 0, max_depth)
def run(input_value: Input) -> Json:
    if input_value["function"] == "message":
        return {"result": extract_message(
            input_value["message"],
            input_value["depth"] if input_value["depth"] is not None else 0,
            input_value["max_depth"] if input_value["max_depth"] is not None else 10,
        ), "logs": []}
    return {"result": extract_response(
        input_value["response"],
        input_value["max_depth"] if input_value["max_depth"] is not None else 10,
    ), "logs": []}
