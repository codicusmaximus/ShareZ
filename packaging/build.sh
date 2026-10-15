#!/bin/sh
# Builds ShareZ in release mode and packages it as .deb, .rpm and an Arch package in dist/.
# Needs nfpm (https://nfpm.goreleaser.com) on PATH. Build on Ubuntu 24.04 so the binary runs on
# every distribution the packages name (see packaging/nfpm.yaml).
set -eu
cd "$(dirname "$0")/.."

SHAREZ_VERSION=$(cargo metadata --no-deps --format-version 1 |
    sed -n 's/.*"name":"sharez","version":"\([^"]*\)".*/\1/p')
export SHAREZ_VERSION

cargo build --release --locked -p sharez
strip target/release/sharez
mkdir -p dist
for packager in deb rpm archlinux; do
    nfpm package --config packaging/nfpm.yaml --packager "$packager" --target dist/
done
ls -l dist/
