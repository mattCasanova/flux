#!/bin/sh
# Flux installer — https://github.com/mattCasanova/flux
#
#   curl -fsSL https://raw.githubusercontent.com/mattCasanova/flux/master/install.sh | sh
#
# Downloads the latest release binary for this machine (macOS or
# Linux, x86_64 or arm64) and installs it to ~/.local/bin (override
# with FLUX_INSTALL_DIR). No Rust toolchain needed.

set -eu

REPO="mattCasanova/flux"

case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    *)
        echo "flux: no prebuilt binary for $(uname -s)." >&2
        echo "Build from source: cargo install --git https://github.com/${REPO} flux-app" >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    arm64 | aarch64) arch="aarch64" ;;
    x86_64) arch="x86_64" ;;
    *)
        echo "flux: unsupported architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

target="${arch}-${os}"
url="https://github.com/${REPO}/releases/latest/download/flux-${target}.tar.gz"
dir="${FLUX_INSTALL_DIR:-$HOME/.local/bin}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading flux (${target})..."
curl -fsSL "$url" -o "$tmp/flux.tar.gz"
tar xzf "$tmp/flux.tar.gz" -C "$tmp"

mkdir -p "$dir"
install -m 755 "$tmp/flux" "$dir/flux"

echo "Installed: $dir/flux"
# (Don't invoke the binary here: releases before v0.4.1 launch the
# app on --version instead of printing.)

case ":$PATH:" in
    *":$dir:"*) ;;
    *)
        echo ""
        echo "Note: $dir is not on your PATH. Add this to your shell rc:"
        echo "  export PATH=\"$dir:\$PATH\""
        ;;
esac
