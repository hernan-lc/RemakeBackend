"""Run with python3 deploy/test_start_cloud.py; no Docker or network needed."""
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest


SCRIPT = Path(__file__).with_name("start-cloud")


class SupervisorTest(unittest.TestCase):
    def test_scanner_failure_stops_api(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            # Use real processes so that the test covers signals and reaping.
            for name in ("clamd", "cloud-api"):
                executable = root / name
                executable.write_text(
                    "#!/usr/bin/env python3\n"
                    "import os, pathlib, time\n"
                    f"pathlib.Path({str(root / (name + '.pid'))!r}).write_text(str(os.getpid()))\n"
                    "time.sleep(60)\n"
                )
                executable.chmod(0o700)
            config = root / "clamd.conf"
            config.touch()
            env = os.environ.copy()
            env.update(
                PATH=f"{root}:{env['PATH']}",
                CLOUD_CLAMD_CONFIG=str(config),
                CLAMAV_ADDR="127.0.0.1:3310",
            )
            process = subprocess.Popen(["bash", str(SCRIPT)], env=env, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 10
                while not all((root / (name + ".pid")).exists() for name in ("clamd", "cloud-api")):
                    self.assertIsNone(process.poll(), "supervisor exited before children started")
                    self.assertLess(time.monotonic(), deadline, "children did not start")
                    time.sleep(0.02)
                scanner = int((root / "clamd.pid").read_text())
                api = int((root / "cloud-api.pid").read_text())
                os.kill(scanner, signal.SIGTERM)
                self.assertNotEqual(process.wait(timeout=10), 0)
                with self.assertRaises(ProcessLookupError):
                    os.kill(api, 0)
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
                if process.stderr:
                    process.stderr.close()


if __name__ == "__main__":
    unittest.main()
