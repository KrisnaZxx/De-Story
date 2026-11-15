"""Private Whisper installation and short-lived audio jobs over SSH."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
import uuid

ROOT = Path.home() / ".local/share/de-story-ssh/whisper"
CACHE = Path.home() / ".cache/de-story-whisper"
CLI = ROOT / "source/build/bin/whisper-cli"
MODEL = ROOT / "ggml-small.bin"
MODEL_BYTES = 487601967
MODEL_SHA256 = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
COMMIT = "d1be6fde11ac6e0407606b4e42fe72d34add8037"  # whisper.cpp v1.9.5


def job_directory(value):
    if str(uuid.UUID(value)) != value:
        raise ValueError("Invalid Whisper job identifier")
    root = CACHE.resolve()
    directory = root / value
    if directory.is_symlink() or directory.resolve().parent != root:
        raise ValueError("Invalid Whisper job directory")
    return directory


def status():
    model = MODEL.is_file() and MODEL.stat().st_size == MODEL_BYTES
    return {"ready": CLI.is_file() and model and bool(shutil.which("ffmpeg")),
            "model": model, "cli": CLI.is_file(), "cpus": os.cpu_count(),
            "freeGb": round(shutil.disk_usage(Path.home()).free / 2**30, 2)}


def install():
    ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (ROOT / "install.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if any(not shutil.which(name) for name in ("cmake", "g++", "git", "ffmpeg")):
            subprocess.run(["sudo", "-n", "apt-get", "update"], check=True)
            subprocess.run(["sudo", "-n", "apt-get", "install", "-y", "cmake", "build-essential", "git", "ffmpeg", "ca-certificates"], check=True)
        if not CLI.is_file():
            source = ROOT / "source"
            if not source.exists():
                subprocess.run(["git", "clone", "--depth", "1", "--branch", "v1.9.5", "https://github.com/ggml-org/whisper.cpp.git", str(source)], check=True)
            actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            if actual != COMMIT:
                raise ValueError("Whisper source revision does not match the pinned release")
            subprocess.run(["cmake", "-S", str(source), "-B", str(source / "build"), "-DCMAKE_BUILD_TYPE=Release", "-DGGML_NATIVE=OFF", "-DWHISPER_BUILD_TESTS=OFF", "-DWHISPER_CURL=OFF"], check=True)
            subprocess.run(["cmake", "--build", str(source / "build"), "--target", "whisper-cli", "-j2"], check=True)
        if not MODEL.exists() or digest(MODEL) != MODEL_SHA256:
            partial = ROOT / "ggml-small.bin.part"
            try:
                request = urllib.request.Request("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin", headers={"User-Agent": "De-Story"})
                with urllib.request.urlopen(request, timeout=120) as response, partial.open("wb") as output:
                    shutil.copyfileobj(response, output, 1024 * 1024)
                if partial.stat().st_size != MODEL_BYTES or digest(partial) != MODEL_SHA256:
                    raise ValueError("Whisper model checksum does not match")
                partial.replace(MODEL)
            finally:
                partial.unlink(missing_ok=True)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def identity(pid):
    proc = Path("/proc") / str(pid)
    return {"pid": pid, "start": (proc / "stat").read_text().split()[21],
            "cwd": str((proc / "cwd").resolve()),
            "program": Path((proc / "cmdline").read_bytes().split(b"\0")[0].decode()).name}


def alive(directory, saved):
    try:
        current = identity(saved["pid"])
        return current == saved and current["cwd"] == str(directory) and current["program"] in ("python3", "ffmpeg", "whisper-cli")
    except (FileNotFoundError, ProcessLookupError, KeyError):
        return False


def record(directory, name, pid):
    temporary = directory / (name + ".tmp")
    temporary.write_text(json.dumps(identity(pid)))
    temporary.replace(directory / name)


def cancel_and_clean(directory):
    for name in ("child.json", "worker.json"):
        path = directory / name
        if not path.exists():
            continue
        saved = json.loads(path.read_text())
        if alive(directory, saved):
            os.kill(saved["pid"], signal.SIGTERM)
            for _ in range(50):
                if not alive(directory, saved):
                    break
                time.sleep(0.1)
            if alive(directory, saved):
                os.kill(saved["pid"], signal.SIGKILL)
    if directory.exists():
        shutil.rmtree(directory)


def execute(directory, args):
    with (directory / "process.log").open("ab") as log:
        process = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        try:
            if process.poll() is None:
                try:
                    record(directory, "child.json", process.pid)
                except FileNotFoundError:
                    if process.poll() is None:
                        raise
            if process.wait() != 0:
                tail = (directory / "process.log").read_bytes()[-1200:].decode(errors="replace")
                raise RuntimeError("Whisper VPS gagal: " + tail)
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            (directory / "child.json").unlink(missing_ok=True)


def run(directory):
    if Path.cwd().resolve() != directory:
        raise ValueError("Wrong Whisper working directory")
    record(directory, "worker.json", os.getpid())
    try:
        if not status()["ready"]:
            raise ValueError("Pasang Whisper VPS melalui Pengaturan terlebih dahulu")
        params = json.loads((directory / "params.json").read_text(encoding="utf-8"))
        language = params["language"]
        if not isinstance(language, str) or not language.isalpha() or len(language) > 8:
            raise ValueError("Invalid Whisper language")
        prompt = params.get("prompt", "")
        if not isinstance(prompt, str) or len(prompt) > 400:
            raise ValueError("Invalid Whisper prompt")
        # One model at a time prevents multiple projects exhausting VPS memory.
        with (ROOT / "run.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            execute(directory, ["ffmpeg", "-hide_banner", "-y", "-i", "audio.input", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", "audio.wav"])
            execute(directory, [str(CLI), "-m", str(MODEL), "-f", "audio.wav", "-l", language,
                                "-ml", "1", "-sow", "-oj", "-of", "output", "-np", "-t",
                                str(max(1, min(8, os.cpu_count() or 1))), "--prompt", prompt])
        output = (directory / "output.json").read_text(encoding="utf-8")
        json.loads(output)
        print(output, flush=True)
    finally:
        shutil.rmtree(directory)


if __name__ == "__main__":
    def interrupted(number, _frame):
        raise SystemExit(128 + number)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    mode = sys.argv[1]
    if mode == "status":
        print(json.dumps(status()))
    elif mode == "install":
        install()
    elif mode == "run":
        run(job_directory(Path.cwd().name))
    elif mode == "clean":
        cancel_and_clean(job_directory(sys.argv[2]))
    else:
        raise ValueError("Unknown Whisper action")
