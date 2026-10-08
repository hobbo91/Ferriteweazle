#!/bin/sh
# Builds every release file of the commit checked out into dist/: the macOS
# disk image here, and at the same time the Windows zips and MSIs and the
# Linux tarballs and AppImages on the machines packaging/release.env names
# (see release.env.example), each into target/release-NAME.log; then
# LGPL-sources-VERSION.tar and SHA256SUMS-VERSION.txt, which the app's Update
# checks downloads against. Publishing is left to you.
set -eu
cd "$(dirname "$0")/.."
[ -z "$(git status --porcelain)" ] || { echo "release: commit first" >&2; exit 1; }
. packaging/release.env
. bundle/greaseweazle.sh
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
# Every machine builds the same gw release, and the Linux packages' AppStream
# release takes the commit's date.
GREASEWEAZLE=$(wanted)
export GREASEWEAZLE
epoch=$(git log -1 --format=%ct)
rm -rf dist
mkdir -p target

# The commit, not the working tree, goes to each machine, and its dist folder
# comes back in a call of its own, clear of what the build prints.
linux() {
    git archive --format=tar HEAD | ssh $LINUX_SSH "mkdir -p $LINUX_DIR && tar -xf - -C $LINUX_DIR"
    ssh $LINUX_SSH "cd $LINUX_DIR && $LINUX_SETUP && export GREASEWEAZLE=$GREASEWEAZLE \
        SOURCE_DATE_EPOCH=$epoch && rm -rf dist && packaging/linux/bundle.sh x86_64 && \
        packaging/linux/bundle.sh aarch64"
    ssh $LINUX_SSH "cd $LINUX_DIR && tar -cf - dist" >target/linux-dist.tar
    tar -xf target/linux-dist.tar
}

# Windows' sshd runs the command with cmd.exe, which ends it at a line break.
windows() {
    git archive --format=tar HEAD | ssh $WINDOWS_SSH "tar -xf - -C $WINDOWS_DIR"
    git_bash="\"C:\\Program Files\\Git\\bin\\bash.exe\" -lc"
    ssh $WINDOWS_SSH "$git_bash \"cd \$(cygpath '$WINDOWS_DIR') && export GREASEWEAZLE=$GREASEWEAZLE && \
        rm -rf dist && packaging/windows/bundle.sh x64 && packaging/windows/bundle.sh arm64\""
    ssh $WINDOWS_SSH "$git_bash \"cd \$(cygpath '$WINDOWS_DIR') && tar -cf - dist\"" >target/windows-dist.tar
    tar -xf target/windows-dist.tar
}

packaging/macos/bundle.sh >target/release-macos.log 2>&1 &
m=$!
linux >target/release-linux.log 2>&1 &
l=$!
windows >target/release-windows.log 2>&1 &
w=$!
failed=
wait $m || failed="$failed macos"
wait $l || failed="$failed linux"
wait $w || failed="$failed windows"
for name in $failed; do
    echo "release: $name failed. The end of target/release-$name.log:" >&2
    tail -n 20 "target/release-$name.log" >&2
done
[ -z "$failed" ]

# The source of the LGPL code in the Linux packages, published with them.
# Named to list after the packages on the release page.
packaging/linux/lgpl-sources.sh "dist/LGPL-sources-$version.tar"
(cd dist && shasum -a 256 Ferriteweazle-* LGPL-sources-* >"SHA256SUMS-$version.txt")
ls -l dist
