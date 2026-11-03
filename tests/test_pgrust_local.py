"""Offline unit tests for the optional pgrust launcher."""
import importlib.util
from io import BytesIO
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "pgrust-local.py"
spec = importlib.util.spec_from_file_location("pgrust_local", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


class LocalPgrustTests(unittest.TestCase):
    def test_platform_linux_x64(self):
        with patch.object(mod.platform, "system", return_value="Linux"), patch.object(mod.platform, "machine", return_value="x86_64"):
            self.assertEqual(mod.platform_name(), "linux-x86_64")

    def test_platform_windows_rejected(self):
        with patch.object(mod.platform, "system", return_value="Windows"):
            with self.assertRaisesRegex(RuntimeError, "WSL2"):
                mod.platform_name()

    def test_database_url_valid(self):
        with patch.dict(mod.os.environ, {"DATABASE_URL": "postgres://cloud:local-only-password@localhost:5433/cloud"}):
            self.assertEqual(mod.database_settings(), ("cloud", "local-only-password", "cloud", 5433))

    def test_database_url_rejects_remote(self):
        with patch.dict(mod.os.environ, {"DATABASE_URL": "postgres://cloud:pass@example.com/cloud"}):
            with self.assertRaisesRegex(RuntimeError, "localhost"):
                mod.database_settings()

    def test_database_url_rejects_injected_identifier(self):
        with patch.dict(mod.os.environ, {"DATABASE_URL": "postgres://cloud:pass@localhost/cloud%22%3Bdrop"}):
            with self.assertRaisesRegex(RuntimeError, "identifiers"):
                mod.database_settings()

    def test_archive_rejects_traversal(self):
        with tempfile.TemporaryFile() as temp:
            with tarfile.open(fileobj=temp, mode="w") as tar:
                info = tarfile.TarInfo("pgrust-0.4-beta-linux-x86_64/../../oops")
                info.size = 1
                tar.addfile(info, BytesIO(b"x"))
            temp.seek(0)
            with tarfile.open(fileobj=temp, mode="r") as tar:
                with self.assertRaisesRegex(RuntimeError, "unsafe path"):
                    mod.validate_archive(tar, "pgrust-0.4-beta-linux-x86_64")

    def test_archive_rejects_symlinks(self):
        with tempfile.TemporaryFile() as temp:
            with tarfile.open(fileobj=temp, mode="w") as tar:
                info = tarfile.TarInfo("pgrust-0.4-beta-linux-x86_64/bin/postgres")
                info.type = tarfile.SYMTYPE
                info.linkname = "/bin/sh"
                tar.addfile(info)
            temp.seek(0)
            with tarfile.open(fileobj=temp, mode="r") as tar:
                with self.assertRaisesRegex(RuntimeError, "link or special"):
                    mod.validate_archive(tar, "pgrust-0.4-beta-linux-x86_64")


if __name__ == "__main__":
    unittest.main()
