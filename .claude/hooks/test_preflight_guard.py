"""Test suite for preflight_guard.py. Exit 2 = blocked, 0 = allowed.

Run: python .claude/hooks/test_preflight_guard.py

Two of these cases exist because the first implementation got them wrong:
  - "clean after ;"  -- plain shlex.split() glues `;` to the preceding word, so
    `pwd; cargo clean` never exposed the clean at a command position. Fixed with
    shlex punctuation_chars=True.
  - "clean as quoted data" -- substring matching blocked commands that merely
    CONTAINED the string, which blocked a legitimate test harness.
"""
import json
import os
import subprocess
import sys

HOOK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "preflight_guard.py")

# Built by concatenation so this file's own source never contains the literal
# that the guard matches on.
CLEAN = "cargo" + " clean"

# Whether a build is live is machine state, not a property of the command, so
# every case except the dedicated deconflict ones runs with that guard disabled.
ENV_NO_DECONFLICT = dict(os.environ, SCM_SKIP_DECONFLICT="1")

CASES = [
    ("plain clean", CLEAN, 2),
    ("clean with --target", CLEAN + " --target aarch64-linux-android", 2),
    ("clean after &&", "ls && " + CLEAN, 2),
    ("clean after inspection verb + &&", "grep -rn x docs/ && " + CLEAN, 2),
    ("clean after ;", "pwd; " + CLEAN, 2),
    ("clean via bash -c", 'bash -c "' + CLEAN + '"', 2),
    ("clean with env prefix", "FOO=1 " + CLEAN, 2),
    ("cargo -C dir clean (flag form)", "cargo -C /tmp clean", 2),
    ("clean as quoted data in python", "python -c \"print('" + CLEAN + "')\"", 0),
    ("clean mentioned in grep only", 'grep -rn "' + CLEAN + '" docs/', 0),
    ("clean mentioned in echo only", 'echo "' + CLEAN + '"', 0),
    ("cargo build --clean is not a clean", "cargo build --clean", 0),
    ("clean_target.sh is allowed", "scripts/clean_target.sh --dry-run --all", 0),
    # A commit message or doc that merely MENTIONS the command is data, not a
    # command. This blocked the hook's own commit before heredocs were stripped.
    ("clean named inside a heredoc body",
     "git commit -F - <<'EOM'\nfix: explain why " + CLEAN + " is banned\nEOM", 0),
    ("real clean after a heredoc still blocked",
     "git commit -F - <<'EOM'\nsome message\nEOM\n" + CLEAN, 2),

    ("agy missing both flags", 'agy -p "do a thing"', 2),
    ("agy missing --model", 'agy --add-dir /repo -p "x"', 2),
    ("agy missing --add-dir", 'agy --model gemini-2.5-pro -p "x"', 2),
    ("agy fully specified", 'agy --add-dir /repo --model gemini-2.5-pro -p "x"', 0),
    ("agy after && still checked", 'ls && agy -p "x"', 2),
    ("agy named in echo only", 'echo "run agy later"', 0),

    ("delegate without --mode", "python scripts/delegate_task.py --task t.md", 2),
    ("delegate with --mode", "python scripts/delegate_task.py --task t.md --mode diff", 0),

    ("unrelated command", "ls -la", 0),
    ("git status", "git status --short", 0),
    ("empty command", "", 0),
    ("whitespace only", "   ", 0),

    # --- Destructive-operation guard -------------------------------------
    # The first four are the VERBATIM commands a concurrent Antigravity session
    # ran on 2026-08-08, in order, each one after being told to stop. They
    # destroyed another session's uncommitted work. Nothing blocked them.
    ("INCIDENT: checkout discarding others' edits",
     "git checkout -- .claude/ CLAUDE.md docs/DOCUMENT_STATUS_INDEX.md", 2),
    ("INCIDENT: powershell recursive force delete",
     'powershell -Command "Remove-Item -Recurse -Force -ErrorAction '
     'SilentlyContinue .claude/hooks/preflight_guard.py, docs/rules"', 2),
    ("INCIDENT: git reset --hard", "git reset --hard 6cb7033a", 2),
    ("INCIDENT: force-push shared branch",
     "git push -f origin tracking/pre-v040-tag-work", 2),

    # Recovery moves that MUST stay allowed -- these are how the incident was
    # undone. A guard that blocks these makes recovery impossible.
    ("RECOVERY: checkout from a ref is allowed",
     "git checkout origin/main -- CLAUDE.md docs/rules .claude/settings.json", 0),
    ("RECOVERY: checkout file from a sha is allowed",
     "git checkout fbb9757d -- HANDOFF/gpt/GPT_MAC_PR139_TAKEOVER_2026-08-07.md", 0),

    ("git clean -fd blocked", "git clean -fd", 2),
    ("git rebase blocked", "git rebase main", 2),
    ("git restore discards, blocked", "git restore core/src/lib.rs", 2),
    ("git restore --staged is safe", "git restore --staged core/src/lib.rs", 0),
    ("git checkout branch switch allowed", "git checkout main", 0),
    ("normal push allowed", "git push origin main", 0),
    ("rm -rf under tmp/ allowed", "rm -rf tmp/scratch", 0),
    ("rm -rf repo path blocked", "rm -rf docs/rules", 2),
    ("rm -f single file allowed", "rm -f tmp/x.log", 0),

    # Escape hatches must work as an INLINE prefix. The hook runs as its own
    # process before the command, so an inline `VAR=1` never reaches its
    # environment -- checking only os.environ made every hatch unusable.
    ("inline override: destructive",
     "SCM_ALLOW_DESTRUCTIVE=1 git reset --hard HEAD", 0),
    ("inline override: cargo clean",
     "SCM_ALLOW_CARGO_CLEAN=1 " + CLEAN, 0),
    ("inline override: agy dispatch",
     'SCM_SKIP_DISPATCH_CHECK=1 agy -p "x"', 0),
    ("bare VAR=1 without the command still blocks",
     "SCM_ALLOW_SOMETHING_ELSE=1 git reset --hard HEAD", 2),
]


def run(cmd, env=ENV_NO_DECONFLICT):
    payload = json.dumps({"tool_name": "Bash", "tool_input": {"command": cmd}})
    p = subprocess.run([sys.executable, HOOK], input=payload,
                       capture_output=True, text=True, env=env)
    return p.returncode


def main():
    extra = []

    p = subprocess.run(
        [sys.executable, HOOK],
        input=json.dumps({"tool_name": "Read", "tool_input": {"file_path": "x"}}),
        capture_output=True, text=True)
    extra.append(("non-Bash tool allowed", p.returncode, 0))

    p = subprocess.run([sys.executable, HOOK], input="not json",
                       capture_output=True, text=True)
    extra.append(("malformed json fails open", p.returncode, 0))

    # Deconflict guard, checked against real machine state in both directions.
    try:
        live = subprocess.run(["tasklist", "/FO", "CSV", "/NH"],
                              capture_output=True, text=True, timeout=10).stdout.lower()
    except Exception:
        live = ""
    building = any(x in live for x in ("cargo.exe", "rustc.exe", "cargo-ndk.exe"))
    extra.append(("deconflict reflects real state (build live=%s)" % building,
                  run("cargo build --workspace", env=dict(os.environ)),
                  2 if building else 0))
    extra.append(("deconflict override respected", run("cargo build --workspace"), 0))

    # -- 2026-08-15 incident: `git checkout <ref> -- .` wiped four files of
    # another session's uncommitted work. The guard used to allow every
    # `checkout <ref> -- <paths>` form as "recovery". It now blocks based on
    # whether the named paths actually HAVE local changes, so recovery keeps
    # working while destruction does not. These two cases pin both halves --
    # the same command shape must flip on working-tree state alone.
    probe = "docs/rules/DELEGATION.md"
    had = os.path.exists(probe)
    original = open(probe, "rb").read() if had else None
    try:
        extra.append(("mass checkout from a ref is blocked",
                      run("git checkout origin/main -- ."), 2))
        if had:
            with open(probe, "ab") as fh:
                fh.write(b"\n<!-- preflight guard test scratch -->\n")
            extra.append(("checkout over a DIRTY path is blocked",
                          run("git checkout origin/main -- " + probe), 2))
            open(probe, "wb").write(original)
            extra.append(("checkout over a CLEAN path stays allowed",
                          run("git checkout origin/main -- " + probe), 0))
    finally:
        if had and original is not None:
            open(probe, "wb").write(original)

    npass = nfail = 0
    results = [(d, run(c), e) for d, c, e in CASES] + extra
    for desc, rc, expected in results:
        ok = rc == expected
        print(("[OK]   " if ok else "[FAIL] ") + desc +
              (" (exit %d)" % rc if ok
               else " (expected %d, got %d)" % (expected, rc)))
        npass, nfail = (npass + 1, nfail) if ok else (npass, nfail + 1)

    print("\npassed: %d  failed: %d" % (npass, nfail))
    sys.exit(1 if nfail else 0)


if __name__ == "__main__":
    main()
