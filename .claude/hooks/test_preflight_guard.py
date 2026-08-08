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
