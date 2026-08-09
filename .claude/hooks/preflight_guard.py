#!/usr/bin/env python
"""PreToolUse hook: block Bash commands that have previously caused expensive,
silent damage in this repo, and print the SOP instead.

Each guard replaces a prose rule that cost standing context tokens on every agent
spawn and could still be forgotten. A hook costs nothing until it fires and
cannot be skipped.

Guards:
  1. cargo clean       -- wipes ALL of target/ regardless of --target (44.7 GB
                          measured loss) and can destroy
                          core/target/generated-sources/.
  2. worker dispatch   -- agy without --add-dir times out; without a pinned
                          --model it can silently spend Anthropic quota;
                          delegate_task.py without --mode silently truncates.
  3. build deconflict  -- concurrent build tools corrupt target/ in ways that
                          read as source errors.
  4. destructive ops   -- `reset --hard`, `checkout -- <paths>`, `restore`,
                          `clean -f`, `rebase`, force-push, and recursive
                          force-deletes outside tmp//target/. security.md
                          already required operator approval for these; on
                          2026-08-08 a concurrent agent ran four of them in
                          sequence and destroyed another session's work.

MATCHING: commands are tokenized with shlex and split into shell segments, and a
guard only fires when the trigger sits at a COMMAND POSITION. Substring matching
was tried first and was wrong in both directions:
  - false positive: `python -c "print('cargo clean')"` blocked, because the
    string appears as data. This blocked a legitimate test harness.
  - false negative: `grep x docs/ && cargo clean` allowed, because the command
    merely STARTED with an inspection verb, so the whole line was skipped.
Segment-aware matching fixes both. `bash -c "..."` payloads are recursed into,
bounded by _MAX_DEPTH.

Contract: exit 0 allows, exit 2 blocks and feeds stderr back to the agent.
Fails OPEN -- any internal error allows the command through, because a broken
hook must never wedge the session. If shlex cannot parse the command, falls back
to conservative substring matching (blocks more, never less).

Escape hatches (deliberate, per-guard). Honoured both from the environment and
as an inline `VAR=1 <command>` prefix -- see override(); checking only
os.environ silently made every hatch unusable, since this hook runs as its own
process before the command does.
  SCM_ALLOW_CARGO_CLEAN=1   skip guard 1
  SCM_SKIP_DISPATCH_CHECK=1 skip guard 2
  SCM_SKIP_DECONFLICT=1     skip guard 3
  SCM_ALLOW_DESTRUCTIVE=1   skip guard 4
"""

import json
import os
import re
import shlex
import subprocess
import sys

_MAX_DEPTH = 3
_SEPARATORS = {"&&", "||", ";", "|", "&", "\n"}
_SHELLS = {"bash", "sh", "zsh", "dash", "ksh", "pwsh", "powershell", "cmd"}

# Paths where recursive deletion is sanctioned (AGENTS.md rule 2: temp files go
# in repo-local tmp/). Everything else needs an operator decision.
_DELETE_OK_PREFIXES = ("tmp/", "./tmp/", "target/", "./target/")

# Commands that read rather than execute. A segment led by one of these is
# skipped -- but ONLY that segment, never the rest of the line.
_INSPECT = {
    "grep", "rg", "echo", "cat", "less", "more", "head", "tail",
    "awk", "sed", "printf", "wc", "find", "ls", "diff",
}

_ENV_ASSIGN = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")

# `cmd <<'MARK' ... MARK` -- the body is DATA, not commands. Without stripping
# it, writing a commit message or doc that merely mentions `cargo clean` trips
# the guard, which blocked this hook's own commit. Verified by test.
_HEREDOC_START = re.compile(r"<<-?\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\1")


def strip_heredocs(command):
    """Remove heredoc bodies and their `<<MARKER` introducers.

    Removing the introducer is what makes this IDEMPOTENT, and it must be: this
    runs both in main() and inside segments(). Keeping the introducer meant a
    second pass saw an orphaned `<<'EOM'` with no terminator left and swallowed
    every following line -- which silently disabled all guards for any command
    that used a heredoc. Verified by test.
    """
    out, marker = [], None
    for line in command.splitlines():
        if marker is not None:
            if line.strip() == marker:
                marker = None
            continue
        found = _HEREDOC_START.search(line)
        if found:
            marker = found.group(2)
            out.append(_HEREDOC_START.sub("", line))
        else:
            out.append(line)
    return "\n".join(out)

# Conservative fallbacks, used only when shlex cannot parse the command.
_RAW_CARGO_CLEAN = re.compile(r"\bcargo\s+clean\b")
_RAW_AGY = re.compile(r"\bagy(\.exe)?\b")
_RAW_DELEGATE = re.compile(r"delegate_task\.py")
_RAW_BUILD = re.compile(
    r"\bcargo\s+(build|test|check|clippy|run)\b|\bgradlew\b|\bcargo-ndk\b"
)

_CARGO_BUILD_VERBS = {"build", "test", "check", "clippy", "run"}

# Process images that indicate a live build. Checked only when a build command
# is about to run, so the tasklist cost is not paid on every Bash call.
#
# Deliberately EXCLUDES java.exe and gradle.exe: the Gradle daemon idles in the
# background for hours after a build finishes, so treating it as a conflict
# would block every subsequent cargo invocation. rustc.exe and cargo-ndk.exe
# only exist during active compilation, which is the condition we care about.
_BUILD_PROCS = ("cargo.exe", "rustc.exe", "cargo-ndk.exe")


def block(message):
    sys.stderr.write(message.strip() + "\n")
    sys.exit(2)


def override(name, raw=""):
    """True if escape hatch `name` is set, in the environment OR inline.

    The inline form matters and is easy to get wrong. This hook runs as its own
    process BEFORE the command does, so `SCM_ALLOW_DESTRUCTIVE=1 git push ...`
    sets the variable for git, never for the hook -- checking only os.environ
    made every documented escape hatch unusable. Verified by test.
    """
    if os.environ.get(name) == "1":
        return True
    return re.search(r"(^|\s)%s=1(\s|$)" % re.escape(name), raw) is not None


def basename(token):
    """Strip directory and .exe so ./scripts/cargo.exe -> cargo."""
    name = re.split(r"[\\/]", token)[-1]
    return name[:-4].lower() if name.lower().endswith(".exe") else name.lower()


def segments(command, depth=0):
    """Split a command line into shell segments of tokens.

    Returns a list of token-lists, each representing one command position.
    Recurses into `bash -c "..."` payloads. Returns None if the command cannot
    be tokenized, signalling the caller to use conservative fallbacks.
    """
    if depth > _MAX_DEPTH:
        return []
    command = strip_heredocs(command)

    # Split on newlines FIRST. shlex treats a newline as ordinary whitespace, so
    # `git commit ... <<EOM ... EOM` followed by a real command on the next line
    # collapsed into one segment led by `git`, hiding the second command from
    # every guard. Verified by test.
    out = []
    for line in command.splitlines():
        if not line.strip():
            continue
        try:
            # punctuation_chars=True is required: plain shlex.split() glues `;`
            # to the preceding word, so `pwd; cargo clean` tokenized as
            # ['pwd;','cargo',...] and the clean was never seen at a command
            # position. Verified by test.
            lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
            lexer.whitespace_split = True
            tokens = list(lexer)
        except ValueError:
            return None
        current = []
        for token in tokens:
            if token in _SEPARATORS:
                if current:
                    out.append(current)
                current = []
            else:
                current.append(token)
        if current:
            out.append(current)

    expanded = []
    for seg in out:
        # Drop leading VAR=value assignments so `FOO=1 cargo clean` still matches.
        while seg and _ENV_ASSIGN.match(seg[0]):
            seg = seg[1:]
        if not seg:
            continue
        expanded.append(seg)
        # Recurse into shell payloads: `bash -c "..."`, and PowerShell's
        # `-Command "..."`. PowerShell matters specifically: the 2026-08-08
        # incident used `powershell -Command "Remove-Item -Recurse -Force ..."`,
        # which hides the destructive verb one level down.
        if basename(seg[0]) in _SHELLS:
            for i, tok in enumerate(seg[1:], start=1):
                if tok.lower() in ("-c", "-command", "/c") and i + 1 < len(seg):
                    nested = segments(seg[i + 1], depth + 1)
                    if nested:
                        expanded.extend(nested)
                    break
    return expanded


def actionable(command):
    """Segments that actually execute something, inspection segments removed."""
    segs = segments(command)
    if segs is None:
        return None
    return [s for s in segs if basename(s[0]) not in _INSPECT]


def is_cargo_clean(seg):
    return basename(seg[0]) == "cargo" and "clean" in seg[1:]


def is_cargo_build(seg):
    if basename(seg[0]) == "cargo":
        return any(v in _CARGO_BUILD_VERBS for v in seg[1:])
    name = basename(seg[0])
    return "gradlew" in name or name == "cargo-ndk"


def is_agy(seg):
    return basename(seg[0]) == "agy"


def is_delegate(seg):
    return any(t.endswith("delegate_task.py") for t in seg)


def guard_cargo_clean(segs, raw):
    if override("SCM_ALLOW_CARGO_CLEAN", raw):
        return
    hit = _RAW_CARGO_CLEAN.search(raw) if segs is None else any(
        is_cargo_clean(s) for s in segs
    )
    if not hit:
        return
    block(
        "[BLOCKED] `cargo clean` is not scoped in this repo.\n"
        "\n"
        "`cargo clean --target <triple>` does NOT clean one triple -- it wipes ALL\n"
        "of target/. Measured: 44.7 GB destroyed when ~4 GB was intended. Running\n"
        "it from inside core/ also destroys core/target/generated-sources/, after\n"
        "which ffi_surface.sh --update reports a vacuous success with no bindings.\n"
        "\n"
        "Use the script, which removes directories by explicit path, backs up\n"
        "generated-sources, and refuses to run while a build tool is live:\n"
        "\n"
        "  scripts/clean_target.sh --dry-run --all   # always look first\n"
        "  scripts/clean_target.sh --triples         # cross-compile outputs only\n"
        "  scripts/clean_target.sh --deps            # intermediates, KEEPS binaries\n"
        "  scripts/clean_target.sh --all\n"
        "\n"
        "Detail: docs/rules/BUILD_AND_CI.md\n"
        "Override (only with a stated reason): SCM_ALLOW_CARGO_CLEAN=1"
    )


def guard_dispatch(segs, raw):
    if override("SCM_SKIP_DISPATCH_CHECK", raw):
        return

    if segs is None:
        agy_segs = [None] if _RAW_AGY.search(raw) else []
        delegate_hit = bool(_RAW_DELEGATE.search(raw)) and "--mode" not in raw
    else:
        agy_segs = [s for s in segs if is_agy(s)]
        delegate_hit = any(is_delegate(s) and "--mode" not in s for s in segs)

    for seg in agy_segs:
        text = raw if seg is None else " ".join(seg)
        missing = []
        if "--add-dir" not in text:
            missing.append(
                "  --add-dir <repo path>  : without it agy re-discovers the repo\n"
                "                           every dispatch and often bails before\n"
                "                           finishing. This is the root cause of\n"
                "                           what looks like a random timeout."
            )
        if "--model" not in text:
            missing.append(
                "  --model '<exact name>' : without an explicit pin, agy can route\n"
                "                           to claude-sonnet/opus and silently spend\n"
                "                           Anthropic quota. Shorthand names\n"
                "                           substitute a different model with no\n"
                "                           error -- use the exact quoted name from\n"
                "                           `agy models`."
            )
        if missing:
            block(
                "[BLOCKED] agy dispatch is missing required flags:\n\n"
                + "\n".join(missing)
                + "\n\nSOP: docs/rules/DELEGATION.md\n"
                "Override: SCM_SKIP_DISPATCH_CHECK=1"
            )

    if delegate_hit:
        block(
            "[BLOCKED] delegate_task.py called without --mode.\n"
            "\n"
            "Full-file mode silently truncates past roughly 300-500 lines, and\n"
            "flash-tier Qwen returns a vacuous success (exit 3) when handed a small\n"
            "edit in full-file form. Pass --mode explicitly:\n"
            "\n"
            "  --mode diff        scoped edits, and anything over ~300 lines\n"
            "  --mode full-file   only for genuinely small whole-file rewrites\n"
            "\n"
            "Also state the wanted output length in the prompt -- max_tokens resolves\n"
            "to the model maximum and is not a brevity constraint.\n"
            "\n"
            "SOP: docs/rules/DELEGATION.md\n"
            "Override: SCM_SKIP_DISPATCH_CHECK=1"
        )


def running_build_processes():
    """Return live build-tool process names, or [] if undetectable."""
    try:
        out = subprocess.run(
            ["tasklist", "/FO", "CSV", "/NH"],
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.lower()
    except Exception:
        return []
    return [p for p in _BUILD_PROCS if p in out]


def guard_deconflict(segs, raw):
    if override("SCM_SKIP_DECONFLICT", raw):
        return
    hit = _RAW_BUILD.search(raw) if segs is None else any(
        is_cargo_build(s) for s in segs
    )
    if not hit:
        return
    live = running_build_processes()
    if not live:
        return
    block(
        "[BLOCKED] a build tool is already running: " + ", ".join(live) + "\n"
        "\n"
        "Multiple agent sessions share this repo. Two concurrent build-tool\n"
        "invocations corrupt target/ in ways that surface as source errors, and\n"
        "Gradle spawns cargo-ndk upstream, so an Android build counts as a cargo\n"
        "build. Wait for the live build, or confirm the process is stale before\n"
        "proceeding -- check real process age, not just output-file existence.\n"
        "\n"
        "Also worth checking before a big build: `df -h /c` (this box runs near\n"
        "97% full; a disk-full failure reads as a code bug).\n"
        "\n"
        "Detail: docs/rules/BUILD_AND_CI.md\n"
        "Override: SCM_SKIP_DECONFLICT=1"
    )


def discards_working_tree(seg):
    """True for `git checkout -- <paths>`, `git checkout .`, bare `git restore`.

    Deliberately ALLOWS `git checkout <ref> -- <paths>`, which restores FROM a
    commit and is the standard recovery move -- it is how the 2026-08-08
    incident was undone.
    """
    if basename(seg[0]) != "git":
        return False
    if "checkout" in seg:
        rest = seg[seg.index("checkout") + 1:]
        rest = [t for t in rest if t == "--" or not t.startswith("-")]
        if rest and rest[0] in ("--", "."):
            return True
    if "restore" in seg and "--staged" not in seg:
        if not any(t == "-s" or t.startswith("--source") for t in seg):
            return True
    return False


def _sanctioned_delete_path(path):
    norm = path.replace("\\", "/").rstrip(",").strip("'\"")
    low = norm.lower()
    if "appdata/local/temp" in low or low.startswith("/tmp/"):
        return True  # scratchpad / system temp, outside the repo
    stripped = norm[2:] if norm.startswith("./") else norm
    return stripped.startswith(_DELETE_OK_PREFIXES) or stripped.startswith(
        ("tmp/", "target/")
    )


def recursive_force_delete(seg):
    """Return risky paths for a recursive+force delete, else None."""
    name = basename(seg[0])
    paths, recursive, force = [], False, False
    if name == "rm":
        for t in seg[1:]:
            if t == "--recursive":
                recursive = True
            elif t == "--force":
                force = True
            elif t.startswith("-") and not t.startswith("--"):
                if "r" in t[1:].lower():
                    recursive = True
                if "f" in t[1:]:
                    force = True
            elif not t.startswith("-"):
                paths.append(t)
    elif name in ("remove-item", "ri", "rd", "rmdir"):
        recursive = name in ("rd", "rmdir")
        for t in seg[1:]:
            low = t.lower().rstrip(",")
            if low.startswith("-recurse"):
                recursive = True
            elif low.startswith("-force"):
                force = True
            elif not t.startswith("-"):
                paths.append(t)
        if name in ("rd", "rmdir"):
            force = True
    else:
        return None
    if not (recursive and force):
        return None
    risky = [p for p in paths if not _sanctioned_delete_path(p)]
    return risky or None


def guard_destructive(segs, raw):
    """Enforce the destructive-operation rules that security.md already states.

    security.md: "Git operations that modify history (rebase, reset,
    force-push) require explicit human trust dialog" and "NEVER execute
    `rm -rf` without explicit human approval". Those were documentation with no
    enforcement until a concurrent agent ran checkout/Remove-Item/reset --hard/
    push -f in sequence on 2026-08-08 and destroyed another session's work.
    """
    if override("SCM_ALLOW_DESTRUCTIVE", raw) or segs is None:
        return

    def fail(title, detail):
        block(
            "[BLOCKED] " + title + "\n\n" + detail + "\n\n"
            "This is an OPERATOR decision, not an agent's. If you are undoing\n"
            "your own mistake, prefer a forward fix: restore from a ref\n"
            "(`git checkout <ref> -- <path>`) or add a revert commit. Never\n"
            "discard state you did not create.\n"
            "\n"
            "Rules: docs/rules/SECURITY_PROTOCOL.md, AGENTS.md rule 5\n"
            "Override: SCM_ALLOW_DESTRUCTIVE=1"
        )

    for seg in segs:
        if basename(seg[0]) == "git":
            if "reset" in seg and "--hard" in seg:
                fail(
                    "`git reset --hard` discards uncommitted work.",
                    "Another agent may have uncommitted changes in this shared\n"
                    "checkout. A hard reset destroys them with no undo, and it\n"
                    "does NOT spare untracked files if paired with a delete.",
                )
            if "rebase" in seg:
                fail(
                    "`git rebase` rewrites history.",
                    "security.md requires an explicit human trust dialog for\n"
                    "history-modifying git operations.",
                )
            if "push" in seg and any(
                t == "-f" or t.startswith("--force") for t in seg
            ):
                fail(
                    "Force-push refused.",
                    "Force-pushing a shared branch -- including the head of an\n"
                    "open pull request -- discards commits other people pushed.\n"
                    "The pre-push hook blocks this for every tool; this is the\n"
                    "earlier, clearer stop.",
                )
            if "clean" in seg and any(
                t.startswith("-") and "f" in t.lower() for t in seg[1:]
            ):
                fail(
                    "`git clean -f` deletes untracked files.",
                    "Untracked files are often another agent's in-progress work\n"
                    "that no commit or reflog can recover.",
                )
            if discards_working_tree(seg):
                fail(
                    "This discards uncommitted working-tree changes.",
                    "`git checkout -- <paths>` / `git restore <paths>` throw away\n"
                    "edits with no undo, including edits made by another session\n"
                    "sharing this checkout.\n"
                    "\n"
                    "To RESTORE a file from a commit instead (allowed):\n"
                    "  git checkout <ref> -- <path>",
                )
        risky = recursive_force_delete(seg)
        if risky:
            fail(
                "Recursive force-delete outside sanctioned paths: "
                + ", ".join(risky[:4]),
                "security.md: never `rm -rf` without explicit human approval.\n"
                "Recursive deletes are permitted under `tmp/`, `target/`, and\n"
                "the session scratchpad only. Untracked files deleted this way\n"
                "are NOT recoverable from git.",
            )


def main():
    try:
        payload = json.load(sys.stdin)
    except Exception:
        sys.exit(0)  # fail open

    if payload.get("tool_name") != "Bash":
        sys.exit(0)

    raw = (payload.get("tool_input") or {}).get("command", "")
    if not raw.strip():
        sys.exit(0)

    try:
        # Strip heredoc bodies for the fallback path too, so an unparseable
        # command does not false-positive on documentation text.
        raw = strip_heredocs(raw)
        segs = actionable(raw)  # None => unparseable, use raw fallbacks
        if segs is not None and not segs:
            sys.exit(0)  # inspection-only command line
        guard_destructive(segs, raw)
        guard_cargo_clean(segs, raw)
        guard_dispatch(segs, raw)
        guard_deconflict(segs, raw)
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)  # fail open

    sys.exit(0)


if __name__ == "__main__":
    main()
