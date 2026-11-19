import sys
from pathlib import Path
from types import ModuleType
from typing import ClassVar, Protocol, Required, Self, TypedDict

from litellm.llms.litellm_proxy.skills.sandbox_executor import SkillsSandboxExecutor
from pydantic import JsonValue, TypeAdapter

MODULE = "llm_sandbox"

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ReplyFields(TypedDict, total=False):
    exit_code: int
    stdout: str | None
    stderr: str | None


class Input(TypedDict, total=False):
    code: Required[str]
    files: dict[str, str]
    requirements: str | None
    image: str | None
    install: ReplyFields
    execution: ReplyFields
    listing: ReplyFields
    runtime_files: dict[str, str]
    refusing_copies: list[str]
    missing_sdk: bool


class SessionEffect(TypedDict):
    lang: str
    verbose: bool
    image: str | None


class CopyEffect(TypedDict):
    sandbox_path: str
    content: str


class Effects:
    def __init__(self) -> None:
        self.session: SessionEffect | None = None
        self.copies: list[CopyEffect] = []
        self.runs: list[str] = []
        self.downloads: list[str] = []
        self.closes: int = 0

    def value(self) -> JsonValue:
        return JSON.validate_python(
            {
                "session": self.session,
                "copies": list(self.copies),
                "runs": list(self.runs),
                "downloads": list(self.downloads),
                "closes": self.closes,
            }
        )


class RunReply:
    def __init__(self, exit_code: int, stdout: str | None, stderr: str | None) -> None:
        self.exit_code = exit_code
        self.stdout = stdout
        self.stderr = stderr


def reply_fields(request: Input, code: str) -> ReplyFields:
    if "os.walk('/sandbox')" in code:
        fields = request.get("listing")
    elif "'pip', 'install', '-r'" in code:
        fields = request.get("install")
    else:
        fields = request.get("execution")
    if fields is None:
        return ReplyFields()
    return fields


class FakeSandboxSession:
    request: ClassVar[Input]
    effects: ClassVar[Effects]

    def __init__(self, *, lang: str, verbose: bool, **extra: str) -> None:
        image = extra.get("image")
        if set(extra) - {"image"}:
            raise TypeError("unexpected sandbox session keywords")
        FakeSandboxSession.effects.session = SessionEffect(
            lang=lang,
            verbose=verbose,
            image=image,
        )

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_args: object) -> None:
        FakeSandboxSession.effects.closes += 1

    def run(self, code: str) -> RunReply:
        FakeSandboxSession.effects.runs.append(code)
        fields = reply_fields(FakeSandboxSession.request, code)
        exit_code = fields.get("exit_code")
        return RunReply(
            exit_code=0 if exit_code is None else exit_code,
            stdout=fields.get("stdout"),
            stderr=fields.get("stderr"),
        )

    def copy_to_runtime(self, source: str, target: str) -> None:
        content = Path(source).read_text(encoding="utf-8")
        FakeSandboxSession.effects.copies.append(
            CopyEffect(sandbox_path=target, content=content)
        )

    def copy_from_runtime(self, source: str, target: str) -> None:
        request = FakeSandboxSession.request
        FakeSandboxSession.effects.downloads.append(source)
        refusing = request.get("refusing_copies")
        if refusing is not None and source in refusing:
            Path(target).write_bytes(b"partial")
            raise RuntimeError("the copy was refused")
        files = request.get("runtime_files")
        if files is None:
            raise FileNotFoundError(source)
        Path(target).write_bytes(files[source].encode())


class FakeSdkModule(ModuleType):
    SandboxSession = FakeSandboxSession


class SandboxExecutor(Protocol):
    def execute(
        self,
        code: str,
        skill_files: dict[str, bytes],
        requirements: str | None = None,
    ) -> dict[str, JsonValue]: ...


def install_sdk(input_value: Input) -> None:
    if input_value.get("missing_sdk"):
        sys.modules[MODULE] = ModuleType(MODULE)
    else:
        sys.modules[MODULE] = FakeSdkModule(MODULE)


def skill_files_of(input_value: Input) -> dict[str, bytes]:
    files = input_value.get("files")
    if files is None:
        return {}
    return {path: content.encode() for path, content in files.items()}


def run(input_value: Input) -> JsonValue:
    effects = Effects()
    FakeSandboxSession.request = input_value
    FakeSandboxSession.effects = effects
    install_sdk(input_value)
    executor: SandboxExecutor = SkillsSandboxExecutor(image=input_value.get("image"))
    result = executor.execute(
        input_value["code"],
        skill_files_of(input_value),
        input_value.get("requirements"),
    )
    return JSON.validate_python({"result": result, "effects": effects.value()})
