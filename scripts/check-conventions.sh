#!/usr/bin/env bash
# Project-wide convention enforcement.
#
# Each check below codifies one project rule. The rationale lives inline
# so the script stands alone — no external doc reference required.
#
#   1. No tokio / async. Async would force a runtime dependency the
#      project deliberately avoids; std::sync::mpsc and OS threads are
#      the chosen primitives.
#   2. No internal-doc references inside files that ship with the repo.
#      Source must explain its own invariants in a comment, not point
#      somewhere else, so a reader who only has the file in front of
#      them can still understand it.
#   3. Atomic writes only. Every write to user state goes through
#      fs_util::atomic_write so a crashed run can't leave a half-written
#      ssh config / cert / preferences file.
#   4. Fault-domain prefixes on error!/warn!. Every surfaced log line is
#      tagged with [external], [config] or [purple] so the user (and the
#      log analyzer in a future incident) can attribute the failure
#      without reading source.
#   5. User-facing message strings separate clauses with periods. Scoped
#      to the string literals in messages.rs and messages/, since code
#      and doc comments never reach the user.
#   6. No hardcoded format!("[A-Z]...") in library modules whose Result
#      chains surface to the user as toasts or CLI eprintln output.
#      Centralization keeps wording consistent and makes future i18n
#      tractable.
#   7. Tests that set the color mode, the theme or the demo flag hold the
#      shared test lock, so they never overlap a parallel render test.
#   8. Only runtime::env::Paths joins `.purple`. Every other module asks
#      it for a path, so a category can move without stragglers.

set -e

FAIL=0

# Strip `#[cfg(test)] mod ... {}` blocks so tests can use direct fs::write,
# tokio (for async test fixtures, if ever needed), etc. without tripping
# the gate. Mirrors the stripper in check-messages.sh — keep them in sync
# if either one grows new exception logic.
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

while IFS= read -r src_file; do
    rel="${src_file#src/}"
    dest="$WORK_DIR/$rel"
    mkdir -p "$(dirname "$dest")"
    awk '
    BEGIN { in_test_attr=0; in_test=0; depth=0 }
    {
        if (in_test) {
            for (i=1; i<=length($0); i++) {
                c=substr($0,i,1)
                if (c=="{") depth++
                else if (c=="}") {
                    depth--
                    if (depth==0) { in_test=0; next }
                }
            }
            next
        }
        if ($0 ~ /^[[:space:]]*#\[cfg\(test\)\]/) {
            in_test_attr=1
            next
        }
        if (in_test_attr && $0 ~ /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+[a-zA-Z_]+/) {
            in_test_attr=0
            in_test=1
            depth=0
            for (i=1; i<=length($0); i++) {
                c=substr($0,i,1)
                if (c=="{") depth++
                else if (c=="}") depth--
            }
            if (depth==0) { in_test=0 }
            next
        }
        if (in_test_attr) { in_test_attr=0 }
        print
    }
    ' "$src_file" > "$dest"
done < <(find src/ -name '*.rs' ! -name '*_tests.rs' ! -name 'tests.rs')

SCAN_ROOT="$WORK_DIR"

# 1. No tokio / async.
HITS=$(grep -rEn '^[[:space:]]*use[[:space:]]+tokio([:;]|$)|tokio::|^[[:space:]]*(pub[[:space:]]+)?async[[:space:]]+fn[[:space:]]' \
    "$SCAN_ROOT" --include='*.rs' || true)
if [ -n "$HITS" ]; then
    echo "ERROR: tokio import or async fn found."
    echo "       Use std::sync::mpsc and OS threads (no async runtime)."
    echo "$HITS" | sed "s|$SCAN_ROOT|src|g"
    FAIL=1
fi

# 2. No internal-doc references in any file that ships with the repo.
#
# Comments in source, scripts and tests must document invariants
# directly, not point at an external doc. `.gitignore` is the only
# allowed exception and lives outside the file-extension filter
# below. The pattern is built from a character class so this very
# script does not contain the literal token it forbids — that would
# be a self-violation the moment grep reads it.
DOC_TOKEN='[C]LAUDE\.md'
DOC_REF_HITS=$(
    {
        grep -rEn "$DOC_TOKEN" src/ --include='*.rs' || true
        grep -rEn "$DOC_TOKEN" scripts/ --include='*.sh' || true
        grep -rEn "$DOC_TOKEN" tests/ --include='*.rs' --include='*.sh' || true
    } | grep -v '^[^:]\+:[0-9]\+:#!/' || true
)
if [ -n "$DOC_REF_HITS" ]; then
    echo "ERROR: Internal-doc reference found."
    echo "       Document the invariant inline so the file stands alone."
    echo "$DOC_REF_HITS"
    FAIL=1
fi

# 3. Atomic writes only.
# fs_util.rs implements atomic_write itself and is the only file allowed
# to call std::fs::write directly. Other modules must funnel through it
# so a crashed write never leaves a partial file on disk.
HITS=$(grep -rEn 'std::fs::write[(]|[^_a-z]fs::write[(]' "$SCAN_ROOT" --include='*.rs' \
    | grep -v '/fs_util\.rs:' || true)
if [ -n "$HITS" ]; then
    echo "ERROR: Direct fs::write found. Use crate::fs_util::atomic_write."
    echo "$HITS" | sed "s|$SCAN_ROOT|src|g"
    FAIL=1
fi

# 4. Fault-domain prefixes on error!/warn!.
# Every surfaced log line begins with [external] (remote/system call
# failures), [config] (user config / on-disk state issues) or [purple]
# (purple's own internal errors). The check is single-line: a multi-line
# error!(\n  "[external] ...") slips through because the prefix is on
# the next line. Single-line is the dominant form; rustfmt collapses
# short calls onto one line. New violations introduced as multi-line
# get caught the moment fmt rejoins them.
#
# `log::error!` and `log::warn!` (fully qualified) are matched too so
# call sites that didn't `use log::*` aren't a back door.
HITS=$(grep -rEn '^[[:space:]]*(log::)?(error|warn)!\("[^[]' "$SCAN_ROOT" --include='*.rs' \
    | grep -v '"\[external\]\|"\[config\]\|"\[purple\]' || true)
if [ -n "$HITS" ]; then
    echo "ERROR: error!/warn! macro without fault-domain prefix."
    echo "       Prefix the literal with [external], [config] or [purple]."
    echo "$HITS" | sed "s|$SCAN_ROOT|src|g"
    FAIL=1
fi

# 5. Clause separation in user-facing message strings.
# Scope: every Rust file under src/messages/, plus the legacy src/messages.rs.
# Comments are skipped: only text the user actually sees is checked.
HITS=$(grep -rn '"[^"]*—[^"]*"' src/messages.rs src/messages/ 2>/dev/null \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
if [ -n "$HITS" ]; then
    echo "ERROR: message string joins clauses with a dash."
    echo "       Use a period instead."
    echo "$HITS"
    FAIL=1
fi

# 6. No hardcoded format!("[A-Z]...") in library modules whose Result
# chains surface to the user as toasts or CLI eprintln output.
#
# These files form the "library modules with known user-error chains":
# clipboard, import, snippet, vault_ssh, containers, key_push. Errors
# they construct via `format!`, `.map_err`, or `.with_context` flow
# unmodified to a `notify_error` toast or `eprintln!("{}", e)`. The
# message-centralization rule strictly scopes to handler/CLI/UI code,
# but the SPIRIT of the rule applies anywhere user-visible strings get
# built — so we extend the gate to these specific files.
#
# Pattern catches the common `format!("Foo bar: {}", x)` form. False
# positives on internal debug strings starting with a capital are rare
# (most internal format! starts with `{`, lowercase, or symbols). Add
# the file to the exclusion list below if a legitimate non-user-facing
# format! gets flagged.
LIBRARY_FILES=(
    "$SCAN_ROOT/clipboard.rs"
    "$SCAN_ROOT/import.rs"
    "$SCAN_ROOT/snippet.rs"
    "$SCAN_ROOT/vault_ssh.rs"
    "$SCAN_ROOT/containers.rs"
    "$SCAN_ROOT/key_push.rs"
)
for f in "${LIBRARY_FILES[@]}"; do
    [ -f "$f" ] || continue
    HITS=$(grep -En 'format!\("[A-Z]' "$f" || true)
    if [ -n "$HITS" ]; then
        rel=$(echo "$f" | sed "s|$SCAN_ROOT|src|g")
        echo "ERROR: Hardcoded user-facing format! in $rel."
        echo "       Move the literal to crate::messages::* and call it from here."
        echo "$HITS" | sed "s|^|$rel:|g"
        FAIL=1
    fi
done

# 7. Tests that flip process-global theme or demo state hold the shared
# test lock.
#
# The color mode, the theme and the demo flag are process globals. A test
# that sets one of them (directly or through `build_demo_app`, which
# enables demo mode) races every parallel test that renders, unless it
# holds `crate::demo_flag::GLOBAL_TEST_LOCK` while it runs. Private
# per-module locks do not serialize against the rest of the suite. The
# check is per file: a test file (or the `#[cfg(test)] mod` region of a
# source file) that calls a mutator must name the shared lock somewhere.
MUTATORS='init_with_mode\(|set_color_mode\(|set_theme\(|demo_flag::enable\(|build_demo_app\(|theme::init\('
while IFS= read -r f; do
    base=$(basename "$f")
    if [[ "$base" == *_tests.rs || "$base" == tests.rs ]]; then
        region=$(cat "$f")
    else
        region=$(awk '
        BEGIN { p=0; pending=0 }
        {
            if (!p) {
                if ($0 ~ /^[[:space:]]*#\[cfg\(test\)\]/) { pending=1; next }
                if (pending) {
                    if ($0 ~ /^[[:space:]]*#\[/) { next }
                    if ($0 ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z_]+/) { p=1; print; next }
                    pending=0
                }
                next
            }
            print
        }' "$f")
    fi
    [ -z "$region" ] && continue
    if echo "$region" | grep -qE "$MUTATORS"; then
        if ! echo "$region" | grep -qE 'GLOBAL_TEST_LOCK|GLOBAL_TEST_IO_LOCK'; then
            echo "ERROR: $f: test code sets the theme, color mode or demo flag without the shared test lock."
            echo "       Hold crate::demo_flag::GLOBAL_TEST_LOCK for the whole test."
            FAIL=1
        fi
    fi
done < <(find src/ -name '*.rs')

# 8. One place knows the on-disk layout.
#
# `runtime::env::Paths` maps every purple file to its category directory
# (config, data, state, cache) and resolves the XDG and PURPLE_*_DIR
# variables. Production code joins `.purple` nowhere else, so a category
# can move without a stray hardcoded path staying behind.
HITS=$(grep -rn '"\.purple"' "$SCAN_ROOT" --include='*.rs' \
    | grep -v '/runtime/env\.rs:' || true)
if [ -n "$HITS" ]; then
    echo "ERROR: hardcoded .purple path outside runtime::env::Paths."
    echo "       Add an accessor to Paths and call that instead."
    echo "$HITS" | sed "s|$SCAN_ROOT|src|g"
    FAIL=1
fi

if [ $FAIL -eq 0 ]; then
    echo "Convention checks: OK"
fi

exit $FAIL
