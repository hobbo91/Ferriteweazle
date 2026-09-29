#!/bin/sh
# Builds every release file of the commit checked out into dist/: the macOS
# disk image here, the Windows zips and MSIs and the Linux tarballs and
# AppImages on the machines packaging/release.env names (see
# release.env.example), then Ferriteweazle-VERSION-SHA256SUMS.txt, which the
# app's Update checks downloads against. Publishing them is left to you.
set -eu
cd "$(dirname "$0")/.."
[ -z "$(git status --porcelain)" ] || { echo "release: commit first" >&2; exit 1; }
. packaging/release.env
. engine/greaseweazle.sh
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
# Every machine builds the same gw release.
GREASEWEAZLE=$(wanted)
export GREASEWEAZLE
rm -rf dist
packaging/macos/bundle.sh

# The commit, not the working tree, goes to each machine. Its dist folder
# comes back as a file, so that a failed build stops the release.
git archive --format=tar HEAD | ssh $LINUX_SSH "mkdir -p $LINUX_DIR && tar -xf - -C $LINUX_DIR"
ssh $LINUX_SSH "cd $LINUX_DIR && $LINUX_SETUP && export GREASEWEAZLE=$GREASEWEAZLE && \
    rm -rf dist && packaging/linux/bundle.sh x86_64 && packaging/linux/bundle.sh aarch64 && \
    tar -cf - dist" >target/linux-dist.tar
tar -xf target/linux-dist.tar

# Windows' sshd runs the command with cmd.exe, which ends it at a line break.
git archive --format=tar HEAD | ssh $WINDOWS_SSH "tar -xf - -C $WINDOWS_DIR"
ssh $WINDOWS_SSH "\"C:\\Program Files\\Git\\bin\\bash.exe\" -lc \"cd \$(cygpath '$WINDOWS_DIR') && \
    export GREASEWEAZLE=$GREASEWEAZLE && rm -rf dist && packaging/windows/bundle.sh x64 && \
    packaging/windows/bundle.sh arm64 && tar -cf - dist\"" >target/windows-dist.tar
tar -xf target/windows-dist.tar

(cd dist && shasum -a 256 Ferriteweazle-* >"Ferriteweazle-$version-SHA256SUMS.txt")
ls -l dist
