#!/usr/bin/env python3
"""Optional, Docker-free pgrust launcher for local RemakeBackend development."""

import argparse
import hashlib
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
from urllib.parse import unquote, urlsplit
from urllib.request import urlopen

VERSION = "0.4-beta"
BASE_URL = f"https://pgrust.com/downloads/v{VERSION}"
PROJECT = Path(__file__).resolve().parent.parent
DEFAULT_HOME = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "remakebackend" / "pgrust"
HOME = Path(os.environ.get("PGRUST_LOCAL_HOME", DEFAULT_HOME)).expanduser().resolve()


def platform_name():
    system = platform.system().lower()
    machine = platform.machine().lower()
    if system not in ("linux", "darwin"):
        raise RuntimeError("pgrust v0.4-beta has no native Windows release; use WSL2 for local development")
    arch = {"x86_64": "x86_64", "amd64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(machine)
    if not arch:
        raise RuntimeError(f"Unsupported pgrust architecture: {machine}")
    if system == "darwin":
        return "macos-arm64" if arch == "aarch64" else "macos-x86_64"
    return f"linux-{arch}"


def release_dir():
    return HOME / "releases" / f"pgrust-{VERSION}-{platform_name()}"


def binaries():
    folder = release_dir() / "bin"
    for name in ("postgres", "initdb", "psql"):
        if not (folder / name).is_file():
            raise RuntimeError(f"Missing {folder / name}; run 'install' first")
    return folder


def download(url, path):
    with urlopen(url, timeout=60) as response, path.open("wb") as target:
        shutil.copyfileobj(response, target)


def validate_archive(archive, expected_root):
    """Reject unsafe archive entries rather than extracting untrusted paths/links."""
    total = 0
    for entry in archive.getmembers():
        path = PurePosixPath(entry.name)
        if path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] != expected_root:
            raise RuntimeError("pgrust archive contains an unsafe path")
        if not (entry.isfile() or entry.isdir()):
            raise RuntimeError("pgrust archive contains a link or special file")
        total += entry.size
        if total > 2 * 1024**3:
            raise RuntimeError("pgrust archive exceeds the extraction size limit")


def install():
    destination = release_dir()
    if (destination / "bin" / "postgres").exists() and (destination / "bin" / "initdb").exists() and (destination / "bin" / "psql").exists():
        print(f"Already installed: {destination}")
        return
    HOME.mkdir(parents=True, exist_ok=True)
    plat = platform_name()
    name = f"pgrust-{VERSION}-{plat}.tar.gz"
    with tempfile.TemporaryDirectory(prefix="pgrust-download-", dir=HOME) as work:
        temp = Path(work)
        archive_path = temp / name
        checksum_path = temp / (name + ".sha256")
        print(f"Downloading official {name} and checksum...")
        download(f"{BASE_URL}/{name}", archive_path)
        download(f"{BASE_URL}/{name}.sha256", checksum_path)
        checksum_text = checksum_path.read_text(encoding="utf-8").strip()
        expected = checksum_text.split()[0].lower()
        if not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise RuntimeError("Invalid upstream SHA-256 sidecar")
        actual = hashlib.sha256(archive_path.read_bytes()).hexdigest()
        if actual != expected:
            raise RuntimeError("pgrust archive checksum mismatch; refusing extraction")
        with tarfile.open(archive_path, "r:gz") as archive:
            validate_archive(archive, f"pgrust-{VERSION}-{plat}")
            archive.extractall(temp)
        extracted = temp / f"pgrust-{VERSION}-{plat}"
        for item in ("bin/postgres", "bin/initdb", "bin/psql"):
            if not (extracted / item).is_file():
                raise RuntimeError(f"Upstream archive is missing {item}")
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            raise RuntimeError(f"Incomplete installation found at {destination}; inspect it manually")
        extracted.rename(destination)
    print(f"Installed {destination}")


def database_settings():
    value = os.environ.get("DATABASE_URL")
    if not value:
        dotenv = PROJECT / ".env"
        if dotenv.is_file():
            for line in dotenv.read_text(encoding="utf-8").splitlines():
                line = line.strip()
                if line.startswith("DATABASE_URL="):
                    value = line.split("=", 1)[1].strip().strip('"\'')
                    break
    if not value:
        raise RuntimeError("Set DATABASE_URL or create .env from .env.example before initializing")
    parsed = urlsplit(value)
    if parsed.scheme not in ("postgres", "postgresql") or parsed.hostname not in ("localhost", "127.0.0.1"):
        raise RuntimeError("Local pgrust requires a postgres:// URL on localhost or 127.0.0.1")
    if not parsed.username or parsed.password is None or not parsed.path.strip("/"):
        raise RuntimeError("DATABASE_URL needs a username, password and database name")
    user, password, database = unquote(parsed.username), unquote(parsed.password), unquote(parsed.path[1:])
    if not all(re.fullmatch(r"[a-z_][a-z0-9_]*", name) for name in (user, database)):
        raise RuntimeError("Use lowercase PostgreSQL identifiers for database and username")
    if not password:
        raise RuntimeError("DATABASE_URL password cannot be empty")
    port = parsed.port or 5432
    if parsed.query or parsed.fragment or not (1 <= port <= 65535):
        raise RuntimeError("Unsupported DATABASE_URL parameters for the local pgrust tool")
    return user, password, database, port


def data_dir():
    return HOME / "data"


def initialize(settings):
    if (data_dir() / "PG_VERSION").is_file():
        return
    if data_dir().exists() and any(data_dir().iterdir()):
        raise RuntimeError(f"Refusing to overwrite a nonempty pgrust directory: {data_dir()}")
    user, password, _, _ = settings
    bin_dir = binaries()
    HOME.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", prefix="pw-", dir=HOME, delete=False) as file:
        pwfile = Path(file.name)
        file.write(password + "\n")
    try:
        pwfile.chmod(0o600)
        subprocess.run([
            str(bin_dir / "initdb"), "-D", str(data_dir()), "--no-locale",
            "--encoding", "UTF8", "-U", user, "--auth-host=scram-sha-256",
            "--auth-local=scram-sha-256", f"--pwfile={pwfile}",
        ], check=True)
    finally:
        pwfile.unlink(missing_ok=True)
    print(f"Initialized private cluster at {data_dir()}")


def pg_env(password):
    return {**os.environ, "PGPASSWORD": password, "RUST_MIN_STACK": "33554432"}


def query(sql, settings, database="postgres"):
    user, password, _, port = settings
    result = subprocess.run([
        str(binaries() / "psql"), "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1",
        "-h", "127.0.0.1", "-p", str(port), "-U", user, "-d", database,
        "-c", sql,
    ], env=pg_env(password), text=True, capture_output=True, timeout=8, check=True)
    return result.stdout.strip()


def port_busy(port):
    with socket.socket() as sock:
        sock.settimeout(0.3)
        return sock.connect_ex(("127.0.0.1", port)) == 0


def verified_pid():
    pidfile = HOME / "pgrust.pid"
    if not pidfile.is_file():
        return None
    try:
        pid = int(pidfile.read_text().strip())
        os.kill(pid, 0)
        process = subprocess.run(["ps", "-p", str(pid), "-o", "command="], capture_output=True, text=True, check=True)
        if str(binaries() / "postgres") not in process.stdout:
            raise RuntimeError(f"PID {pid} does not belong to this pgrust installation; refusing to signal it")
        return pid
    except (ValueError, ProcessLookupError, subprocess.CalledProcessError):
        pidfile.unlink(missing_ok=True)
        return None


def raise_stack():
    import resource
    current, hard = resource.getrlimit(resource.RLIMIT_STACK)
    target = 65520 * 1024
    if hard != resource.RLIM_INFINITY and hard < target:
        raise RuntimeError("The hard stack limit is below the 65520 KiB pgrust requirement")
    resource.setrlimit(resource.RLIMIT_STACK, (max(current, target), hard))


def up():
    settings = database_settings()
    _, _, database, port = settings
    install()
    initialize(settings)
    if verified_pid():
        print("pgrust already running")
        return
    if port_busy(port):
        raise RuntimeError(f"Port {port} already in use; refusing to connect to an unrelated server")
    rundir = HOME / "run"
    rundir.mkdir(parents=True, exist_ok=True)
    log = HOME / "pgrust.log"
    with log.open("ab") as output:
        process = subprocess.Popen([
            str(binaries() / "postgres"), "-D", str(data_dir()), "-k", str(rundir),
            "-p", str(port), "-c", "listen_addresses=127.0.0.1",
            "-c", "io_method=sync", "-c", "max_stack_depth=60000",
        ], stdout=output, stderr=subprocess.STDOUT, env=pg_env(settings[1]),
            start_new_session=True, preexec_fn=raise_stack)
    (HOME / "pgrust.pid").write_text(str(process.pid))
    for _ in range(80):
        if process.poll() is not None:
            (HOME / "pgrust.pid").unlink(missing_ok=True)
            raise RuntimeError(f"pgrust exited during startup; inspect {log}")
        try:
            version = query("SELECT version()", settings)
            if "pgrust" not in version.lower():
                raise RuntimeError("Connected database does not identify as pgrust")
            break
        except subprocess.CalledProcessError:
            time.sleep(0.25)
    else:
        raise RuntimeError(f"Timed out waiting for pgrust; inspect {log}")
    exists = query(f"SELECT 1 FROM pg_database WHERE datname = '{database}'", settings)
    if exists != "1":
        query(f'CREATE DATABASE "{database}"', settings)
    print(f"pgrust running on 127.0.0.1:{port} (pid {process.pid})")
    print("Next: cargo run -p cloud-api -- migrate && cargo run -p cloud-api")


def down():
    pid = verified_pid()
    if not pid:
        print("pgrust is not running (managed process)")
        return
    os.kill(pid, signal.SIGINT)
    for _ in range(80):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            (HOME / "pgrust.pid").unlink(missing_ok=True)
            print("pgrust stopped; data retained")
            return
        time.sleep(0.25)
    raise RuntimeError("Server did not stop within 20 seconds; inspect its log and PID")


def status():
    pid = verified_pid()
    print(f"managed pgrust: {'running (pid ' + str(pid) + ')' if pid else 'stopped'}")
    print(f"data: {data_dir()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("install", "up", "down", "status"))
    args = parser.parse_args()
    try:
        {"install": install, "up": up, "down": down, "status": status}[args.command]()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"error: {exc}\n")


if __name__ == "__main__":
    main()
