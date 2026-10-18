#!/usr/bin/env bash
# Type-checks the Windows-only code (win/, pabulong-setup) on macOS or Linux,
# with no Windows target or SDK installed.
#
# The `windows` crates compile to nothing off Windows (`#![cfg(windows)]`).
# This copies the workspace and those crates (from the local cargo registry,
# fetching them first if needed) to a temp dir, switches their platform
# gates on, and runs `cargo check` for the host. It proves the code agrees
# with the real windows-rs API signatures; it does not link or run anything,
# and struct layouts are the host's (the same as ARM64 Windows on Apple
# silicon). CI on Windows remains the real build.
#
#   windows/scripts/typecheck-off-windows.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

(cd "$ROOT/windows" && cargo fetch --locked >/dev/null)
REG="$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/ | head -1)"

mkdir -p "$TMP/windows"
cp -R "$ROOT/rust" "$TMP/rust"
rm -rf "$TMP/rust/target"
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$TMP/"
(cd "$ROOT/windows" && tar --exclude ./target --exclude ./driver --exclude ./out -cf - .) | (cd "$TMP/windows" && tar -xf -)

version() { sed -n "/^name = \"$1\"$/{n;s/version = \"\(.*\)\"/\1/p;}" "$ROOT/windows/Cargo.lock" | head -1; }
PATCHES=""
for crate in windows windows-core windows-strings windows-result windows-collections windows-numerics windows-threading; do
  v="$(version "$crate")"
  [ -n "$v" ] || continue
  cp -R "$REG/$crate-$v" "$TMP/p-$crate"
  rm -f "$TMP/p-$crate/.cargo-ok"
  PATCHES="$PATCHES$crate = { path = \"../p-$crate\" }"$'\n'
done
# windows-future only builds on Windows and none of its APIs are used.
fv="$(version windows-future)"
if [ -n "$fv" ]; then
  mkdir -p "$TMP/stub-future/src"
  printf '[package]\nname = "windows-future"\nversion = "%s"\nedition = "2021"\n[features]\ndefault = ["std"]\nstd = []\n' "$fv" > "$TMP/stub-future/Cargo.toml"
  : > "$TMP/stub-future/src/lib.rs"
  PATCHES="${PATCHES}windows-future = { path = \"../stub-future\" }"$'\n'
fi
printf '\n[patch.crates-io]\n%s' "$PATCHES" >> "$TMP/windows/Cargo.toml"

python3 -I - "$TMP" <<'PY'
import glob, os, re, sys
os.chdir(sys.argv[1])
attr = re.compile(r'(#!?\[cfg(?:_attr)?\(.*\)\]|cfg!\(.*?\))')
def on(m):
    t = re.sub(r'\bnot\(windows\)', 'any()', m.group(0))
    return re.sub(r'(?<![\w])windows(?![\w])', 'all()', t)
for p in glob.glob('p-*/src/**/*.rs', recursive=True):
    s = open(p).read()
    if p.startswith('p-windows/src/') and p.endswith('lib.rs'):
        s = s.replace('#![cfg(windows)]', '')
    n = attr.sub(on, s)
    if 'p-windows-strings' in p and p.endswith('hstring.rs'):
        n = n.replace('feature = "std"', 'feature = "off_windows"')  # std::os::windows
    if n != s:
        open(p, 'w').write(n)
for p in glob.glob('windows/crates/*/Cargo.toml'):
    s = open(p).read()
    open(p, 'w').write(s.replace("[target.'cfg(windows)'.dependencies.windows]", "[dependencies.windows]"))
for p in glob.glob('windows/crates/*/src/**/*.rs', recursive=True):
    s = open(p).read()
    s = s.replace('#[cfg(windows)]', '#[cfg(all())]').replace('#[cfg(not(windows))]', '#[cfg(any())]')
    s = s.replace('#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]', '')
    open(p, 'w').write(s)
PY

cd "$TMP/windows"
cargo check --offline -p pabulong -p pabulong-setup --message-format short 2>&1 | grep -E '^crates/' || true
cargo check --offline -p pabulong -p pabulong-setup >/dev/null 2>&1 && echo "Windows code type-checks." || { echo "Windows code has errors (see above)."; exit 1; }
