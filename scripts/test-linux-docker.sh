#!/usr/bin/env bash
# Build Laralah and run its test suite inside a Linux (Debian) container.
#
#   scripts/test-linux-docker.sh          # cargo build + cargo test
#   scripts/test-linux-docker.sh bash     # interactive shell in the same environment
#
# The source tree is mounted read-only; build output and the cargo registry live in
# named Docker volumes so repeat runs are incremental.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE="laralah-linux-test:latest"

docker build -t "$IMAGE" -f - "$ROOT_DIR/scripts" <<'DOCKERFILE'
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config clang cmake \
        libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
        libx11-dev libx11-xcb-dev libxcb1-dev libxi-dev libxrandr-dev \
        libfontconfig-dev libfreetype-dev libasound2-dev libssl-dev libzstd-dev \
        php-cli openssh-client \
        libsecret-tools gnome-keyring dbus dbus-x11 \
    && rm -rf /var/lib/apt/lists/*
DOCKERFILE

CMD=(bash -c 'set -euo pipefail
  rustc --version
  cargo build --locked
  cargo test --locked
  # Exercise the Secret Service path: a throwaway D-Bus session + unlocked gnome-keyring.
  dbus-run-session -- bash -c "
    echo -n laralah-test | gnome-keyring-daemon --unlock --components=secrets >/dev/null
    cargo test --locked secret_service -- --ignored --nocapture
  "')
if [[ "${1:-}" == "bash" ]]; then
  CMD=(bash)
fi

TTY_FLAGS=()
[[ -t 0 && -t 1 ]] && TTY_FLAGS=(-it)

docker run --rm ${TTY_FLAGS[@]+"${TTY_FLAGS[@]}"} \
  -v "$ROOT_DIR":/src:ro \
  -v laralah-linux-target:/target \
  -v laralah-linux-cargo:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  -w /src \
  "$IMAGE" "${CMD[@]}"
