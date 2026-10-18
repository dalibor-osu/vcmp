#!/usr/bin/env bash
# Builds the Linux and Windows release archives on this machine, in Docker, for when
# the Release workflow can't run (README, "Building the archives locally").
#
# The build runs in Ubuntu 22.04 like the workflow's Linux runner, so the Linux
# program runs on glibc 2.35 or newer; built on the host, it would need the host's
# glibc. Windows is cross-built there with MinGW-w64 (x86_64-pc-windows-gnu), where
# the workflow builds with MSVC on Windows. The archives, named and laid out as the
# workflow's, and their SHA256SUMS.txt land in dist/.
#
# The integration keys are passed into the build when they are set here, as the
# workflow passes its secrets. Without them the build has no Last.fm and no "Find
# photo online".

set -euo pipefail

keys=(VCMP_LASTFM_API_KEY VCMP_LASTFM_SHARED_SECRET VCMP_FANARTTV_API_KEY)

if [ "${1:-}" = --in-container ]; then
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
    linux="vcmp-$version-linux-x64"
    windows="vcmp-$version-windows-x64"
    for target in x86_64-unknown-linux-gnu x86_64-pc-windows-gnu; do
        cargo build --release --locked -p vcmp-app --target "$target"
    done

    # A folder of the program and the licence texts per archive.
    stage="$CARGO_TARGET_DIR/stage"
    rm -rf "$stage"
    for folder in "$linux" "$windows"; do
        mkdir -p "$stage/$folder"
        cp LICENSE-MIT LICENSE-APACHE "$stage/$folder/"
        cp assets/fonts/sora/OFL.txt "$stage/$folder/Sora-OFL.txt"
    done
    cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/vcmp" "$stage/$linux/"
    cp "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/vcmp.exe" "$stage/$windows/"

    mkdir -p dist
    rm -f "dist/$linux.tar.gz" "dist/$windows.zip"
    tar -C "$stage" -czf "dist/$linux.tar.gz" "$linux"
    (cd "$stage" && zip -qr "/src/dist/$windows.zip" "$windows")
    (cd dist && sha256sum "$linux.tar.gz" "$windows.zip" | tee SHA256SUMS.txt)
    exit
fi

cd "$(dirname "$0")/.."
image=vcmp-release-local
docker build --tag "$image" - < packaging/release-local.Dockerfile

# An empty key counts as not set, as in the workflow.
pass=()
for name in "${keys[@]}"; do
    if [ -n "${!name:-}" ]; then
        pass+=(--env "$name")
        echo "$name is set"
    fi
done

# Its own target directory and Cargo cache: the host's builds are for another glibc
# and compiler. Run as this user, so that what it writes stays this user's.
mkdir -p target/release-local/cargo-home
docker run --rm --user "$(id -u):$(id -g)" \
    --volume "$PWD:/src" --workdir /src \
    --env CARGO_HOME=/src/target/release-local/cargo-home \
    --env CARGO_TARGET_DIR=/src/target/release-local \
    "${pass[@]}" "$image" bash packaging/release-local.sh --in-container
