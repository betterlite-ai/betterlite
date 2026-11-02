import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import isolate as isolation
import warm_inputs as warm
from environment import PROJECT

HERE = Path(__file__).parent


def git(root: Path, *arguments: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(root), *arguments], stderr=subprocess.PIPE)


def repository(root: Path) -> None:
    root.mkdir()
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.name", "Fixture")
    git(root, "config", "user.email", "fixture@example.com")
    git(root, "remote", "add", "origin", str(root))
    (root / "counter.rs").write_text("pub fn count() -> u32 { 1 }\n")
    git(root, "add", "counter.rs")
    git(root, "commit", "-qm", "initial counter")


class IsolationTests(unittest.TestCase):
    def test_an_unfinished_goose_merge_is_preserved_for_recovery(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            source, target = Path(folder) / "source", Path(folder) / "B126"
            repository(source)
            git(source, "worktree", "add", "-qb", "task/B126-goose", str(target))
            (source / "counter.rs").write_text("goose on main\n")
            git(source, "commit", "-qam", "main goose")
            (target / "counter.rs").write_text("goose on task\n")
            git(target, "commit", "-qam", "task goose")
            with self.assertRaises(subprocess.CalledProcessError):
                git(target, "merge", "main")
            old_git = Path(git(target, "rev-parse", "--absolute-git-dir").decode().strip())
            with self.assertRaisesRegex(RuntimeError, "unfinished Git operation"):
                isolation.isolate(source, target, "task/B126-goose")
            self.assertTrue((target / ".git").is_file())
            self.assertTrue((old_git / "MERGE_HEAD").is_file())

    def test_a_retained_task_branch_keeps_its_unmerged_goose_commit(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            source, target = Path(folder) / "source", Path(folder) / "B125"
            repository(source)
            git(source, "switch", "-qc", "task/B125-goose")
            (source / "goose.txt").write_text("unmerged honk\n")
            git(source, "add", "goose.txt")
            git(source, "commit", "-qm", "teach goose to honk")
            head = git(source, "rev-parse", "HEAD")
            git(source, "switch", "-q", "main")
            isolation.isolate(source, target, "task/B125-goose")
            self.assertEqual(git(target, "rev-parse", "HEAD"), head)
            self.assertEqual((target / "goose.txt").read_text(), "unmerged honk\n")
            self.assertEqual(git(target, "config", "user.name"), b"Fixture\n")

    def test_private_metadata_preserves_commits_staging_and_untracked_work(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            source, target = Path(folder) / "source", Path(folder) / "B123"
            repository(source)
            git(source, "worktree", "add", "-qb", "task/B123-counter", str(target))
            (target / "counter.rs").write_text("pub fn count() -> u32 { 2 }\n")
            git(target, "add", "counter.rs")
            (target / "counter.rs").write_text("pub fn count() -> u32 { 3 }\n")
            (target / "notes.txt").write_text("unfinished experiment\n")
            before = git(target, "status", "--porcelain=v1", "-z", "--untracked-files=all")
            isolation.isolate(source, target, "task/B123-counter")
            self.assertTrue((target / ".git").is_dir())
            self.assertEqual(before, git(target, "status", "--porcelain=v1", "-z", "--untracked-files=all"))
            self.assertIn(b"{ 2 }", git(target, "show", ":counter.rs"))
            self.assertIn("{ 3 }", (target / "counter.rs").read_text())
            other = Path(folder) / "other"
            git(source, "worktree", "add", "-qb", "other", str(other))
            git(source, "worktree", "remove", str(other))
            git(source, "gc", "--prune=now")
            self.assertIn(b"{ 1 }", git(target, "show", "HEAD:counter.rs"))
            self.assertNotEqual(
                git(target, "rev-parse", "--git-common-dir"),
                git(source, "rev-parse", "--path-format=absolute", "--git-common-dir"),
            )

    def test_a_live_checkout_cannot_be_migrated(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            source, target = Path(folder) / "source", Path(folder) / "B124"
            repository(source)
            git(source, "worktree", "add", "-qb", "task/B124-counter", str(target))
            child = subprocess.Popen(["sleep", "60"], cwd=target)
            try:
                with self.assertRaisesRegex(RuntimeError, "still used"):
                    isolation.isolate(source, target, "task/B124-counter")
                self.assertTrue((target / ".git").is_file())
            finally:
                child.terminate()
                child.wait(timeout=5)


class WarmInputsTests(unittest.TestCase):
    def test_rust_changes_do_not_rewarm_answers_but_helper_and_case_changes_do(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder) / "repo"
            repository(root)
            initial = warm.fingerprint(root, "HEAD")
            (root / "counter.rs").write_text("pub fn count() -> u32 { 2 }\n")
            git(root, "commit", "-qam", "change Rust")
            self.assertEqual(initial, warm.fingerprint(root, "HEAD"))
            helper = root / "tools/oracle/helpers/counter.py"
            helper.parent.mkdir(parents=True)
            helper.write_text("def run(value): return value + 1\n")
            git(root, "add", str(helper))
            git(root, "commit", "-qm", "change helper")
            changed = warm.fingerprint(root, "HEAD")
            self.assertNotEqual(initial, changed)
            case = root / "tools/oracle/cases/counter.yaml"
            case.parent.mkdir(parents=True)
            case.write_text("input: 2\n")
            git(root, "add", str(case))
            git(root, "commit", "-qm", "new case")
            self.assertNotEqual(changed, warm.fingerprint(root, "HEAD"))


class WatcherTests(unittest.TestCase):
    def watch(self, gate_conclusion: str) -> tuple[subprocess.CompletedProcess[str], str]:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            gh = root / "gh"
            program = root / "fake-gh.py"
            program.write_text("""import os, sys
from pathlib import Path
a = sys.argv[1:]
with open(os.environ['CALL_LOG'], 'a') as log: log.write(' '.join(a) + '\\n')
if a[:2] == ['auth', 'token']: print('fixture')
elif a[:2] == ['pr', 'view']: print('task/B123-counter')
elif a[:2] == ['api', 'graphql']: print('MERGED CLEAN false false ' + '1' * 40)
elif a[:2] == ['run', 'list'] and '--event' in a:
    if '--workflow' not in a: print('100,' + '1' * 40 + ',completed,success')
    else: print('200,' + '1' * 40 + ',completed,' + os.environ['GATE_CONCLUSION'])
""")
            gh.write_text(f'#!/bin/sh\nexec uv run --locked --project "{PROJECT}" python "{program}" "$@"\n')
            gh.chmod(0o755)
            fake_git = root / "git"
            fake_git.write_text("#!/bin/sh\nexit 0\n")
            fake_git.chmod(0o755)
            env = dict(
                os.environ,
                PATH=str(root) + os.pathsep + os.environ["PATH"],
                BETTERLITE_ROOT=str(root),
                CALL_LOG=str(root / "calls"),
                GATE_CONCLUSION=gate_conclusion,
            )
            result = subprocess.run(
                ["bash", str(HERE / "pr-watch.sh"), "123"],
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            return result, (root / "calls").read_text()

    def test_pages_success_does_not_hide_a_failed_gate(self) -> None:
        result, calls = self.watch("failure")
        self.assertEqual(result.returncode, 1)
        self.assertIn("main run 200 failure", result.stdout)
        self.assertIn("--workflow gate.yml", calls)

    def test_success_names_the_gate_run(self) -> None:
        result, _ = self.watch("success")
        self.assertEqual(result.returncode, 0)
        self.assertIn("main run 200 passed", result.stdout)
        self.assertNotIn("run 100", result.stdout)


if __name__ == "__main__":
    unittest.main()
