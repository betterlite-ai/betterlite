import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from environment import PROJECT, PYTHON
from test_ops import git, repository

SOURCE = Path(__file__).resolve().parents[2]
BINARY = SOURCE / ".development/gate/target/debug/gate"


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))


def fixture(root: Path) -> dict[str, str]:
    repository(root)
    development = root / ".development"
    shutil.copytree(
        SOURCE / ".development/ops",
        development / "ops",
        ignore=shutil.ignore_patterns("__pycache__", ".venv", ".ruff_cache"),
    )
    shutil.copytree(SOURCE / ".development/schema", development / "schema")
    (root / "tools/oracle/litellm").mkdir(parents=True)
    (root / "tools/oracle/litellm/.keep").touch()
    write_json(development / "rules.yaml", {"rules": []})

    def command(name: str, run: str) -> dict[str, str]:
        return {"id": name, "why": "toy check", "run": run}

    write_json(
        development / "checks.yaml",
        {
            "tools": [],
            "settings": {
                "always_in_scope": [],
                "budget_excludes": [],
                "scope_roots": [],
                "file_size": {"roots": [], "src_max_lines": 300, "test_max_lines": 1000},
            },
            "base": {
                "builtin": [],
                "commands": [
                    command("fmt", "true"),
                    command("tests", 'echo tests >> "$TRACE"; test "$(cat counter)" = 2'),
                ],
            },
            "optional": [],
        },
    )
    write_json(
        development / "tasks/B123-goose.yaml",
        {
            "id": "B123",
            "title": "Goose",
            "description": "Count two silly geese.",
            "scope": {"paths": ["counter"], "lint_tables": False},
            "references": [],
            "dod": [],
            "checks": {"optional": [], "commands": []},
            "oracle_cases": [],
            "deviations": [],
            "out_of_scope": [],
            "pitfalls": [],
            "budget": {"max_changed_lines": 20, "max_new_expects": 0},
            "review_level": "standard",
        },
    )
    write_json(
        development / "backlog/B123.yaml",
        {
            "id": "B123",
            "title": "Goose",
            "milestone": "toy",
            "status": "todo",
            "depends_on": [],
            "file": "tasks/B123-goose.yaml",
            "covers": [],
            "parent": None,
            "split_from": None,
            "resolution": None,
            "superseded_by": None,
            "blocked_reason": None,
            "pr": None,
        },
    )
    role = {
        "instructions": "roles/toy.md",
        "ladder": [{"runner": "toy", "model": "local-script", "effort": "none", "attempts": 1, "timeout_minutes": 1}],
    }
    policy = {
        "review_by_other_vendor": False,
        "max_worker_attempts": 1,
        "limit_retry_minutes": 30,
        "max_parallel": 2,
        "overflow": False,
        "max_fix_turns": 2,
        "max_review_rounds": 3,
        "max_blocked_reports": 2,
        "max_hook_blocks": 6,
        "writable": ["{git_common_dir}"],
    }
    write_json(
        development / "agents.yaml",
        {
            "worktrees": "../worktrees",
            "vendors": [{"id": "toy", "limit_markers": ["quota"], "max_parallel": 2}],
            "runners": [
                {
                    "id": "toy",
                    "vendor": "toy",
                    "program": "uv",
                    "args": [*PYTHON[1:], str(root.parent / "fake.py"), "{output}"],
                    "output": "file",
                }
            ],
            "roles": {name: role for name in ["worker", "reviewer", "security_reviewer", "parity_reviewer", "planner"]},
            "policy": policy,
        },
    )
    (root / "counter").write_text("0")
    git(root, "add", ".development", "counter", "tools")
    git(root, "commit", "-qm", "toy dispatch configuration")
    git(root, "update-ref", "refs/remotes/origin/main", "HEAD")
    fake = root.parent / "fake.py"
    fake.write_text("""import json, os, subprocess, sys
from pathlib import Path
sys.stdin.read()
output = Path(sys.argv[1])
trace = Path(os.environ["TRACE"])
def record(value):
    with trace.open("a") as stream: stream.write(value + "\\n")
if "reviewer" in output.name:
    record("review")
    answer = {"task":"B123", "verdict":"approve", "findings":[]}
else:
    number = 1 + (trace.read_text().count("worker\\n") if trace.exists() else 0)
    record("worker")
    Path("counter").write_text(str(max(1, number - 1)))
    subprocess.run(["git", "add", "counter"], check=True)
    subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.com",
                    "commit", "--allow-empty", "-qm", "count geese"], check=True)
    summary = "short" if number == 1 else "The goose counter now records the requested two geese."
    answer = {"task":"B123", "status":"done", "summary":summary, "blocker":"", "owned":[], "prior_art":[], "notes":[]}
    payload = {"cwd":str(Path.cwd()), "last_assistant_message":json.dumps(answer)}
    subprocess.run([os.environ["TOY_GATE"], "hook"], input=json.dumps(payload),
                    text=True, check=True, capture_output=True)
output.write_text(json.dumps(answer))
""")
    tools = root.parent / "bin"
    tools.mkdir()
    uv = tools / "uv"
    uv.write_text(
        '#!/bin/sh\nif [ "$1" = sync ]; then echo preparation >> "$TRACE"; exit 0; fi\n'
        'shift 4\nexec "$REAL_UV" run --locked --project "$TOY_PINNED_PROJECT" "$@"\n'
    )
    uv.chmod(0o755)
    return {
        **os.environ,
        "REAL_UV": str(shutil.which("uv")),
        "TOY_PINNED_PROJECT": str(PROJECT),
        "TRACE": str(root.parent / "trace"),
        "TOY_GATE": str(BINARY),
        "PATH": str(tools) + os.pathsep + os.environ["PATH"],
    }


class DispatchTests(unittest.TestCase):
    def test_report_and_hook_do_not_bypass_the_final_gate_or_repeat_its_expensive_checks(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder) / "main"
            environment = fixture(root)
            result = subprocess.run(
                [str(BINARY), "dispatch", "B123"],
                cwd=root,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
                timeout=30,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            events = (Path(folder) / "trace").read_text().splitlines()
            self.assertEqual(events.count("worker"), 3)
            self.assertEqual(events.count("tests"), 2, events)
            self.assertEqual(events[-1], "review")
            self.assertEqual(events.count("review"), 1)
            history = list((root / ".git/betterlite/B123/gate/runs").glob("*/result.json"))
            self.assertEqual(len(history), 4)  # Two hooks plus two final gates.

    def test_unavailable_vendor_never_prepares_a_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder) / "main"
            environment = fixture(root)
            marker = root / ".git/betterlite/limits/toy"
            marker.parent.mkdir(parents=True)
            marker.write_text("Not logged in")
            result = subprocess.run(
                [str(BINARY), "dispatch", "B123"],
                cwd=root,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
                timeout=10,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("every worker vendor is unavailable", result.stderr)
            self.assertFalse((Path(folder) / "trace").exists())
            self.assertFalse((Path(folder) / "worktrees").exists())

    def test_comma_separated_check_selection_runs_both_checks(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder) / "main"
            environment = fixture(root)
            (root / "counter").write_text("2")
            result = subprocess.run(
                [str(BINARY), "run", "B123", "--only", "fmt,tests"],
                cwd=root,
                env=environment,
                text=True,
                capture_output=True,
                check=False,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("PASS fmt", result.stdout)
            self.assertIn("PASS tests", result.stdout)


if __name__ == "__main__":
    unittest.main()
