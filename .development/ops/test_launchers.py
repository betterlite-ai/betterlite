import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from environment import PROJECT
from test_ops import git, repository

OPS = Path(__file__).resolve().parent


class LauncherTests(unittest.TestCase):
    def test_maintenance_stops_launchers_before_their_tools_run(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            marker = root / "main/.git/betterlite/maintenance"
            marker.parent.mkdir(parents=True)
            marker.touch()
            environment = {
                **os.environ,
                "BETTERLITE_ROOT": str(root / "main"),
                "BETTERLITE_WORKTREES": str(root / "worktrees"),
                "BETTERLITE_OPS": str(OPS),
            }
            for name in ("dispatch-flow.sh", "next-slot.sh", "oracle-warmup.sh", "feeder.sh"):
                with self.subTest(launcher=name):
                    result = subprocess.run(
                        ["bash", str(OPS / name), "B123", "toy"],
                        env=environment,
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=5,
                    )
                    self.assertEqual(result.returncode, 75, result.stdout + result.stderr)
                    self.assertIn("maintenance is active", result.stderr)

    def test_failed_warmup_is_retried_and_success_survives_a_restart(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worktrees = root / "worktrees"
            worktrees.mkdir()
            warm = worktrees / "oracle-warmup"
            repository(warm)
            git(warm, "update-ref", "refs/remotes/origin/main", "HEAD")
            logs = worktrees / "logs"
            logs.mkdir()
            tools = root / "bin"
            tools.mkdir()
            for name, script in {
                "free": "echo 'Mem: 58 1 1 1 1 20'",
                "df": "echo 'Use%'; echo '10%'",
                "sleep": 'touch "$BETTERLITE_WORKTREES/logs/oracle-warmup.STOP"',
                "systemd-run": 'echo build >> "$BETTERLITE_WORKTREES/logs/builds"; exit "$WARM_EXIT"',
            }.items():
                executable = tools / name
                executable.write_text("#!/bin/sh\n" + script + "\n")
                executable.chmod(0o755)
            pinned = root / "main/.development/ops"
            pinned.parent.mkdir(parents=True)
            pinned.symlink_to(PROJECT, target_is_directory=True)
            environment = {
                **os.environ,
                "HOME": str(root),
                "BETTERLITE_ROOT": str(root / "main"),
                "BETTERLITE_WORKTREES": str(worktrees),
                "BETTERLITE_OPS": str(OPS),
                "PATH": str(tools) + os.pathsep + os.environ["PATH"],
            }
            for code in ("1", "0", "0"):
                (logs / "oracle-warmup.STOP").unlink(missing_ok=True)
                subprocess.run(
                    ["bash", str(OPS / "oracle-warmup.sh")],
                    env={**environment, "WARM_EXIT": code},
                    check=True,
                    capture_output=True,
                    timeout=10,
                )
                self.assertEqual((logs / "oracle-warmup.inputs").exists(), code == "0")
            self.assertEqual((logs / "builds").read_text().splitlines(), ["build", "build"])


if __name__ == "__main__":
    unittest.main()
