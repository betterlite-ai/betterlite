from typing_extensions import TypedDict


# ast-grep-ignore: no-long-names
class CodeInterpreterInterceptionConfig(TypedDict, total=False):
    enabled: bool
    enabled_providers: list[str]
    sandbox_tool_name: str
