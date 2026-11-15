#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
test "$(uname -s)" != MINGW64_NT || exit 1
cache=.toolchain/whisper-unix
mkdir -p "$cache" resources/whisper-unix
if [ ! -d "$cache/source/.git" ]; then
  git clone https://github.com/ggml-org/whisper.cpp.git "$cache/source"
fi
git -C "$cache/source" fetch --depth 1 origin d1be6fde11ac6e0407606b4e42fe72d34add8037
git -C "$cache/source" checkout --detach d1be6fde11ac6e0407606b4e42fe72d34add8037
cmake -S "$cache/source" -B "$cache/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF -DWHISPER_BUILD_EXAMPLES=ON
cmake --build "$cache/build" --target whisper-cli --parallel 2
install -m755 "$cache/build/bin/whisper-cli" resources/whisper-unix/whisper-cli
resources/whisper-unix/whisper-cli --help >/dev/null 2>&1
