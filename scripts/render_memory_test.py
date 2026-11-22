"""Linux audio render regression, using graphs emitted by the native test and synthetic tones."""
import array
import json
import math
from pathlib import Path
import resource
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import wave


def tone(path, hz, seconds):
    with wave.open(str(path), "wb") as out:
        out.setparams((1, 2, 48000, 0, "NONE", "not compressed"))
        out.writeframes(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * hz * i / 48000))) for i in range(round(seconds * 48000))))


def render(root, graph, inputs, destination, raw=False):
    (root / "graph.txt").write_text(graph)
    args = ["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-filter_complex_threads", "2", "-y"]
    for name in inputs:
        args += ["-threads", "1", "-i", name]
    args += ["-filter_complex_script", "graph.txt", "-map", "[voice]", "-ar", "48000", "-ac", "1", "-f", "f32le" if raw else "null", destination]

    def limit():
        resource.setrlimit(resource.RLIMIT_STACK, (512 * 1024, 512 * 1024))
        resource.setrlimit(resource.RLIMIT_AS, (1536 * 2**20, 1536 * 2**20))

    peak = 0
    with (root / "stderr.txt").open("w+") as errors:
        process = subprocess.Popen(args, cwd=root, stdout=subprocess.DEVNULL, stderr=errors, preexec_fn=limit)
        deadline = time.monotonic() + 60
        try:
            while process.poll() is None:
                if time.monotonic() > deadline:
                    raise TimeoutError("Synthetic audio render exceeded 60 seconds")
                try:
                    status = (Path("/proc") / str(process.pid) / "status").read_text()
                    peak = max(peak, int(next(line.split()[1] for line in status.splitlines() if line.startswith("VmRSS:"))))
                except (OSError, StopIteration):
                    pass
                time.sleep(.01)
        finally:
            if process.poll() is None:
                process.kill()
            code = process.wait()
        errors.seek(0)
        return code, peak / 1024, errors.read()


def main():
    assert sys.platform.startswith("linux") and shutil.which("ffmpeg"), "Requires Linux FFmpeg"
    graphs = json.loads(Path(sys.argv[1]).read_text())
    with tempfile.TemporaryDirectory(prefix="bara-memory-check-") as temporary:
        root = Path(temporary)
        tone(root / "tone.wav", 440, .15)
        tone(root / "second.wav", 880, .2)
        old = [f"[{i}:a]aresample=48000,aformat=channel_layouts=stereo,adelay={i * 10000}|{i * 10000}[a{i}]" for i in range(60)]
        old += ["".join(f"[a{i}]" for i in range(60)) + "amix=inputs=60:normalize=0:dropout_transition=0[voice]"]
        code, peak, error = render(root, ";".join(old), ["tone.wav"] * 60, "-")
        print(f"Reference delay graph: exit {code}, observed RSS {peak:.1f} MiB.")
        code, peak, error = render(root, graphs["long"], ["tone.wav"] * 60, "-")
        assert code == 0, error
        assert peak < 512, f"Narration consumed {peak:.1f} MiB"
        print(f"New 10-minute / 60-clip graph passed; peak observed RSS {peak:.1f} MiB.")
        code, _, error = render(root, graphs["short"], ["tone.wav", "second.wav"], "short.raw", raw=True)
        assert code == 0, error
        samples = array.array("f", (root / "short.raw").read_bytes())
        assert abs(len(samples) / 48000 - .9) < .002
        for start, end, audible in ((.04, .12, True), (.22, .28, False), (.34, .45, False), (.53, .65, True), (.76, .88, False)):
            values = samples[round(start * 48000):round(end * 48000)]
            rms = math.sqrt(sum(value * value for value in values) / len(values))
            assert (rms > .05) if audible else (rms < 1e-5), (start, end, rms)
        print("Timing passed: first voice, padded silence, clip without voice, second voice, and final silence.")


if __name__ == "__main__":
    main()
