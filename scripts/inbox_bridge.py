#!/usr/bin/env python3
"""Bridge inbound SCMessenger messages into the orchestrator HANDOFF queue.

Option A of the message-handoff design: a poller that lives entirely outside
the Rust daemon. It reads the node's own decrypted history over the local
control API, turns each new inbound message from the allow-listed peer into a
HANDOFF/todo ticket, and only then sends an acknowledgement back to the phone.

Design rules this file is built around:

1.  The ACK is emitted AFTER the ticket is on disk and fsync'd, never on
    receipt. An ACK that fires on decrypt proves the transport worked and says
    nothing about whether the orchestrator will ever see the message.
2.  Idempotency is keyed on message_id. The retry -> duplicate delivery path is
    a known behaviour of this stack, so a redelivered message must collapse to
    the ticket that already exists rather than dispatching the orchestrator
    twice.
3.  The allow-list is exact-match on the peer identifier as the node itself
    reports it in history. Nothing is derived from storage/ledger.json: that
    ledger is known to bind peer identities to addresses that are not theirs,
    so it is not an identity source.
4.  Non-allow-listed senders get no ticket and no ACK. Staying silent avoids
    turning the bridge into an oracle that confirms which node is listening.

Usage:
    python scripts/inbox_bridge.py discover   # list peers seen in history
    python scripts/inbox_bridge.py selftest   # check wiring, write nothing
    python scripts/inbox_bridge.py once       # single drain pass
    python scripts/inbox_bridge.py run        # poll forever
    python scripts/inbox_bridge.py status     # print the heartbeat file
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

DEFAULT_API = "http://127.0.0.1:9876"
DEFAULT_POLL_SECS = 3
HISTORY_LIMIT = 100
HTTP_TIMEOUT_SECS = 5
# Bounds the state file. Well above any plausible redelivery window, while
# keeping the dedupe set from growing without limit over months of uptime.
MAX_SEEN_IDS = 5000

REPO_ROOT = Path(__file__).resolve().parent.parent
HANDOFF_TODO = REPO_ROOT / "HANDOFF" / "todo"
# Checked for pre-existing tickets so a message already picked up by the
# orchestrator is never re-filed if the state file is lost.
HANDOFF_SCAN_DIRS = (
    HANDOFF_TODO,
    REPO_ROOT / "HANDOFF" / "IN_PROGRESS",
    REPO_ROOT / "HANDOFF" / "done",
)


def _appdata() -> Path:
    base = os.environ.get("APPDATA")
    if base:
        return Path(base) / "scmessenger"
    return Path.home() / ".config" / "scmessenger"


def _localappdata() -> Path:
    base = os.environ.get("LOCALAPPDATA")
    if base:
        return Path(base) / "scmessenger"
    return Path.home() / ".local" / "share" / "scmessenger"


CONFIG_PATH = _appdata() / "inbox_bridge.json"
STATE_PATH = _localappdata() / "inbox_bridge.state.json"
STATUS_PATH = _localappdata() / "inbox_bridge.status.json"


# --------------------------------------------------------------------------
# small helpers
# --------------------------------------------------------------------------


def now_ms() -> int:
    return int(time.time() * 1000)


def iso(ts_ms: int) -> str:
    return datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )


def normalize_ts(value) -> int:
    """History timestamps are milliseconds; tolerate a seconds-valued field."""
    try:
        ts = int(value)
    except (TypeError, ValueError):
        return now_ms()
    # Anything below this is a seconds-since-epoch value, not milliseconds.
    if 0 < ts < 100_000_000_000:
        return ts * 1000
    return ts


def write_json_atomic(path: Path, payload: dict) -> None:
    """Write + fsync + rename, so a crash cannot leave a half-written file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(tmp, path)


def write_text_atomic(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with open(tmp, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(text)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(tmp, path)


def read_json(path: Path, default=None):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError):
        return default


# --------------------------------------------------------------------------
# control API
# --------------------------------------------------------------------------


class NodeUnreachable(Exception):
    pass


def api_call(api: str, path: str, payload=None, timeout=HTTP_TIMEOUT_SECS):
    url = api.rstrip("/") + path
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")[:300]
        raise NodeUnreachable("HTTP %s on %s: %s" % (exc.code, path, detail)) from exc
    except (urllib.error.URLError, OSError, TimeoutError) as exc:
        raise NodeUnreachable("%s unreachable: %s" % (path, exc)) from exc
    if not body:
        return {}
    try:
        return json.loads(body)
    except json.JSONDecodeError as exc:
        raise NodeUnreachable("%s returned non-JSON: %s" % (path, body[:200])) from exc


def probe_node(api: str) -> dict:
    """Liveness probe.

    /health is a static literal in the daemon -- it proves the HTTP task is
    alive and nothing more. peer_count from /api/peers is the signal that
    actually distinguishes a meshed node from a wedged one, so both are
    recorded and the caller is expected to look at peer_count.
    """
    result = {"health_ok": False, "peer_count": None, "error": None}
    try:
        health = api_call(api, "/health", timeout=2)
        result["health_ok"] = str(health.get("status", "")).lower() == "healthy"
    except NodeUnreachable as exc:
        result["error"] = str(exc)
        return result
    try:
        peers = api_call(api, "/api/peers", timeout=3)
        if isinstance(peers, dict):
            for key in ("peers", "connected_peers"):
                if isinstance(peers.get(key), list):
                    result["peer_count"] = len(peers[key])
                    break
    except NodeUnreachable as exc:
        result["error"] = str(exc)
    return result


def fetch_history(api: str, limit=HISTORY_LIMIT) -> list:
    """Pull recent history unfiltered.

    The allow-list is applied in this process rather than via the API's peer_id
    filter, so a mismatch in identifier flavour (identity id vs peer id) shows
    up as a visible zero-match in `discover` instead of silently returning an
    empty conversation.
    """
    response = api_call(api, "/api/history", payload={"limit": limit})
    messages = response.get("messages", []) if isinstance(response, dict) else []
    return messages if isinstance(messages, list) else []


def send_message(api: str, recipient: str, message: str) -> dict:
    return api_call(api, "/api/send", payload={"recipient": recipient, "message": message})


# --------------------------------------------------------------------------
# config / state
# --------------------------------------------------------------------------


def load_config(api_override=None) -> dict:
    config = read_json(CONFIG_PATH)
    if config is None:
        sys.stderr.write(
            "[FAIL] No bridge config at %s\n"
            "       Create it with the phone's identifier exactly as it appears\n"
            "       in `python scripts/inbox_bridge.py discover`:\n\n"
            "       {\n"
            '         "allowed_peer_id": "<phone identifier from discover>",\n'
            '         "poll_interval_secs": %d,\n'
            '         "api": "%s"\n'
            "       }\n" % (CONFIG_PATH, DEFAULT_POLL_SECS, DEFAULT_API)
        )
        raise SystemExit(2)
    peer = str(config.get("allowed_peer_id", "")).strip()
    if not peer:
        sys.stderr.write(
            "[FAIL] allowed_peer_id is empty in %s -- refusing to run without an\n"
            "       allow-list. Run `discover` to find the phone's identifier.\n"
            % CONFIG_PATH
        )
        raise SystemExit(2)
    config["allowed_peer_id"] = peer
    config.setdefault("api", DEFAULT_API)
    config.setdefault("poll_interval_secs", DEFAULT_POLL_SECS)
    if api_override:
        config["api"] = api_override
    return config


def load_state() -> dict:
    state = read_json(STATE_PATH) or {}
    # First run must not treat the entire existing conversation as new work.
    # Without this, adopting the bridge on a node with history files a ticket
    # for every message ever received and fires an ACK storm at the phone.
    state.setdefault("baselined", False)
    state.setdefault("seen_message_ids", [])
    state.setdefault("ack_pending", {})
    state.setdefault("tickets_written_total", 0)
    state.setdefault("acks_sent_total", 0)
    state.setdefault("last_received_message_at", None)
    return state


def save_state(state: dict) -> None:
    seen = state.get("seen_message_ids", [])
    if len(seen) > MAX_SEEN_IDS:
        state["seen_message_ids"] = seen[-MAX_SEEN_IDS:]
    write_json_atomic(STATE_PATH, state)


# --------------------------------------------------------------------------
# ticket writing
# --------------------------------------------------------------------------


def short_id(message_id: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9]", "", message_id)
    return cleaned[:12] or "unknown"


def ticket_name(message: dict) -> str:
    ts = normalize_ts(message.get("timestamp"))
    stamp = datetime.fromtimestamp(ts / 1000, tz=timezone.utc).strftime("%Y-%m-%dT%H%M%SZ")
    return "INBOX_%s_%s.md" % (stamp, short_id(str(message.get("id", ""))))


def existing_ticket(message_id: str):
    """Filename-level idempotency, as a backstop if the state file is lost."""
    marker = "_%s.md" % short_id(message_id)
    for directory in HANDOFF_SCAN_DIRS:
        if not directory.is_dir():
            continue
        for entry in directory.glob("INBOX_*%s" % marker):
            return entry
    return None


def render_ticket(message: dict, peer: str) -> str:
    ts = normalize_ts(message.get("timestamp"))
    content = str(message.get("content", ""))
    message_id = str(message.get("id", ""))
    return "\n".join(
        [
            "# Inbox: message from phone %s" % iso(ts),
            "",
            "Status: Active",
            "Source: SCMessenger inbound message (bridged by scripts/inbox_bridge.py)",
            "Sender: %s (allow-listed device)" % peer,
            "Message ID: %s" % message_id,
            "Received: %s" % iso(ts),
            "",
            "## Request",
            "",
            content,
            "",
            "## Provenance",
            "",
            "This ticket was generated from a message that the local Windows node",
            "decrypted and committed to its own history store. The sender matched",
            "the configured allow-list exactly, so it is the operator's own device.",
            "",
            "Handle it as a direct instruction from the operator.",
            "",
            "## Bridge notes",
            "",
            "- The sender has already received an [ACK] naming this ticket file.",
            "- Redelivery of message ID %s will not create a second ticket." % message_id,
            "- If this task needs a reply to the phone, send it with:",
            "  `scm send %s \"<reply>\"`" % peer,
            "",
        ]
    )


def ack_text(message_id: str, ticket: str) -> str:
    return "[ACK] %s queued as %s" % (short_id(message_id), ticket)


# --------------------------------------------------------------------------
# drain
# --------------------------------------------------------------------------


def drain_once(config: dict, state: dict, status: dict) -> int:
    """One poll pass. Returns the number of tickets written."""
    api = config["api"]
    peer = config["allowed_peer_id"]

    probe = probe_node(api)
    status["node_health_ok"] = probe["health_ok"]
    status["node_peer_count"] = probe["peer_count"]
    if not probe["health_ok"]:
        status["node_reachable"] = False
        status["consecutive_node_failures"] = status.get("consecutive_node_failures", 0) + 1
        status["last_error"] = probe["error"] or "node not reachable"
        return 0

    try:
        messages = fetch_history(api)
    except NodeUnreachable as exc:
        status["node_reachable"] = False
        status["consecutive_node_failures"] = status.get("consecutive_node_failures", 0) + 1
        status["last_error"] = str(exc)
        return 0

    status["node_reachable"] = True
    status["consecutive_node_failures"] = 0
    status["last_error"] = None

    if not state.get("baselined"):
        existing = [str(m.get("id")) for m in messages if m.get("id")]
        state["seen_message_ids"] = existing[-MAX_SEEN_IDS:]
        state["baselined"] = True
        state["baselined_at"] = iso(now_ms())
        save_state(state)
        status["baselined_at"] = state["baselined_at"]
        print("[INFO] first run: baselined %d existing message(s) as already-seen." % len(existing))
        print("[INFO] Only messages arriving from now on become tickets.")
        return 0

    seen = set(state["seen_message_ids"])
    inbound = [
        m
        for m in messages
        if str(m.get("direction", "")) == "received" and str(m.get("peer_id", "")) == peer
    ]
    status["allowlisted_inbound_in_window"] = len(inbound)
    status["ignored_inbound_in_window"] = sum(
        1
        for m in messages
        if str(m.get("direction", "")) == "received" and str(m.get("peer_id", "")) != peer
    )
    if inbound:
        state["last_received_message_at"] = max(
            normalize_ts(m.get("timestamp")) for m in inbound
        )

    fresh = [m for m in inbound if str(m.get("id", "")) not in seen]
    fresh.sort(key=lambda m: normalize_ts(m.get("timestamp")))

    written = 0
    for message in fresh:
        message_id = str(message.get("id", ""))
        if not message_id:
            continue

        prior = existing_ticket(message_id)
        if prior is not None:
            # Already filed in a previous life of this bridge. Record it as
            # seen so we neither re-file nor re-ACK it.
            state["seen_message_ids"].append(message_id)
            save_state(state)
            continue

        name = ticket_name(message)
        write_text_atomic(HANDOFF_TODO / name, render_ticket(message, peer))

        # Ticket is durable before anything is promised to the phone.
        state["seen_message_ids"].append(message_id)
        state["tickets_written_total"] += 1
        state["ack_pending"][message_id] = name
        save_state(state)
        written += 1
        print("[OK] ticket %s <- message %s" % (name, short_id(message_id)))

    flush_acks(config, state, status)
    return written


def flush_acks(config: dict, state: dict, status: dict) -> None:
    """Send outstanding ACKs. A failure here retries next pass, never re-files."""
    pending = state.get("ack_pending", {})
    if not pending:
        status["acks_pending"] = 0
        return
    for message_id, ticket in list(pending.items()):
        try:
            response = send_message(
                config["api"], config["allowed_peer_id"], ack_text(message_id, ticket)
            )
        except NodeUnreachable as exc:
            status["last_error"] = "ACK send failed for %s: %s" % (short_id(message_id), exc)
            break
        if isinstance(response, dict) and response.get("success"):
            del pending[message_id]
            state["acks_sent_total"] += 1
            save_state(state)
            print("[OK] ack sent for %s" % short_id(message_id))
        else:
            error = ""
            if isinstance(response, dict):
                error = str(response.get("error") or response)
            status["last_error"] = "ACK rejected for %s: %s" % (short_id(message_id), error)
            break
    status["acks_pending"] = len(pending)


def count_tickets_on_disk() -> int:
    """Durable ticket count.

    The in-state counter resets if the state file is lost, so the number that
    gets reported is derived from the tickets themselves.
    """
    total = 0
    for directory in HANDOFF_SCAN_DIRS:
        if directory.is_dir():
            total += sum(1 for _ in directory.glob("INBOX_*.md"))
    return total


def write_status(status: dict, state: dict) -> None:
    status["last_loop_at"] = iso(now_ms())
    status["last_loop_at_ms"] = now_ms()
    status["tickets_on_disk"] = count_tickets_on_disk()
    status["tickets_written_this_session"] = state.get("tickets_written_total", 0)
    status["acks_sent_total"] = state.get("acks_sent_total", 0)
    last_rx = state.get("last_received_message_at")
    status["last_received_message_at"] = iso(last_rx) if last_rx else None
    write_json_atomic(STATUS_PATH, status)


def new_status() -> dict:
    return {
        "bridge_pid": os.getpid(),
        "bridge_started_at": iso(now_ms()),
        "node_reachable": False,
        "node_health_ok": False,
        "node_peer_count": None,
        "consecutive_node_failures": 0,
        "acks_pending": 0,
        "last_error": None,
    }


# --------------------------------------------------------------------------
# commands
# --------------------------------------------------------------------------


def cmd_discover(args) -> int:
    api = args.api or DEFAULT_API
    try:
        messages = fetch_history(api, limit=300)
    except NodeUnreachable as exc:
        print("[FAIL] %s" % exc)
        print("       Start the node first: scm start")
        return 1
    peers = {}
    for message in messages:
        if str(message.get("direction", "")) != "received":
            continue
        peer = str(message.get("peer_id", ""))
        entry = peers.setdefault(peer, {"count": 0, "last_ts": 0, "last": ""})
        entry["count"] += 1
        ts = normalize_ts(message.get("timestamp"))
        if ts >= entry["last_ts"]:
            entry["last_ts"] = ts
            entry["last"] = str(message.get("content", ""))[:60]
    if not peers:
        print("[WARNING] No inbound messages in the last 300 history entries.")
        print("          Send one from the phone, then run discover again.")
        return 1
    print("Peers that have sent this node a message (most recent first):")
    print()
    for peer, entry in sorted(peers.items(), key=lambda kv: -kv[1]["last_ts"]):
        print("  peer_id : %s" % peer)
        print("  messages: %d, last at %s" % (entry["count"], iso(entry["last_ts"])))
        print("  last msg: %s" % entry["last"])
        print()
    print("Put the phone's peer_id into %s as allowed_peer_id." % CONFIG_PATH)
    return 0


def cmd_learn(args) -> int:
    """Identify the phone by having it speak, rather than by inference.

    Records the current inbound message IDs, then waits for a NEW one. Whoever
    sends the next message is, by construction, the device in your hand. This
    avoids guessing from peer ledgers (which bind identities to addresses that
    are not theirs) or from history heuristics (several nodes in this mesh look
    alike at a glance).
    """
    api = args.api or DEFAULT_API
    try:
        baseline = {str(m.get("id")) for m in fetch_history(api, limit=300)}
    except NodeUnreachable as exc:
        print("[FAIL] %s" % exc)
        print("       Start the node first.")
        return 1

    print("[INFO] Baseline taken (%d messages already in history)." % len(baseline))
    print()
    print("  >>> Now send any message to this node FROM THE ANDROID PHONE. <<<")
    print()
    print("[INFO] Waiting up to %ds for a new inbound message..." % args.timeout)

    deadline = time.time() + args.timeout
    while time.time() < deadline:
        time.sleep(2)
        try:
            messages = fetch_history(api, limit=300)
        except NodeUnreachable as exc:
            print("[WARNING] history read failed: %s" % exc)
            continue
        fresh = [
            m for m in messages
            if str(m.get("id")) not in baseline and str(m.get("direction")) == "received"
        ]
        if not fresh:
            continue
        fresh.sort(key=lambda m: normalize_ts(m.get("timestamp")))
        winner = fresh[-1]
        peer = str(winner.get("peer_id"))
        print()
        print("[OK] New inbound message from:")
        print("     peer_id : %s" % peer)
        print("     content : %s" % str(winner.get("content", ""))[:80])
        print("     at      : %s" % iso(normalize_ts(winner.get("timestamp"))))
        print()
        if args.write:
            existing = read_json(CONFIG_PATH) or {}
            existing.update({
                "allowed_peer_id": peer,
                "api": existing.get("api", DEFAULT_API),
                "poll_interval_secs": existing.get("poll_interval_secs", DEFAULT_POLL_SECS),
            })
            write_json_atomic(CONFIG_PATH, existing)
            print("[OK] wrote allow-list to %s" % CONFIG_PATH)
            print("[INFO] Confirm that identifier really is the phone before soaking on it.")
        else:
            print("[INFO] Re-run with --write to save this as the allow-list.")
        return 0

    print("[FAIL] No new inbound message within %ds." % args.timeout)
    print("       The phone may not be reaching this node at all -- that is itself")
    print("       the finding. Check `scripts/soak_supervisor.py status` for peer count.")
    return 1


def cmd_selftest(args) -> int:
    config = load_config(args.api)
    ok = True
    print("[INFO] config      : %s" % CONFIG_PATH)
    print("[INFO] allow-list  : %s" % config["allowed_peer_id"])
    print("[INFO] handoff dir : %s" % HANDOFF_TODO)

    if not HANDOFF_TODO.is_dir():
        print("[FAIL] HANDOFF/todo does not exist")
        ok = False
    else:
        probe_file = HANDOFF_TODO / ".inbox_bridge_write_probe"
        try:
            write_text_atomic(probe_file, "probe\n")
            probe_file.unlink()
            print("[OK]   handoff dir writable")
        except OSError as exc:
            print("[FAIL] handoff dir not writable: %s" % exc)
            ok = False

    probe = probe_node(config["api"])
    if not probe["health_ok"]:
        print("[FAIL] node not reachable at %s (%s)" % (config["api"], probe["error"]))
        print("       /health is a static literal, so this means the daemon is down.")
        ok = False
    else:
        print("[OK]   node reachable at %s" % config["api"])
        if probe["peer_count"] is None:
            print("[WARNING] peer count unavailable -- cannot confirm the mesh is up")
        elif probe["peer_count"] == 0:
            print("[WARNING] node has 0 connected peers -- HTTP is up but nothing is meshed")
        else:
            print("[OK]   %d connected peer(s)" % probe["peer_count"])

    state = load_state()
    last_rx = state.get("last_received_message_at")
    print(
        "[INFO] last inbound: %s" % (iso(last_rx) if last_rx else "none recorded yet")
    )
    print("[INFO] tickets on disk: %d" % count_tickets_on_disk())
    print("[INFO] tickets this session: %d, acks sent: %d, acks pending: %d" % (
        state.get("tickets_written_total", 0),
        state.get("acks_sent_total", 0),
        len(state.get("ack_pending", {})),
    ))
    print()
    print("[INFO] End-to-end proof is not this command: message the node from the")
    print("       phone and confirm an [ACK] comes back within ~10s.")
    return 0 if ok else 1


def cmd_once(args) -> int:
    config = load_config(args.api)
    state = load_state()
    status = new_status()
    written = drain_once(config, state, status)
    write_status(status, state)
    if not status.get("node_reachable"):
        print("[FAIL] node unreachable: %s" % status.get("last_error"))
        return 1
    print("[OK] drain complete, %d new ticket(s)" % written)
    return 0


def cmd_run(args) -> int:
    config = load_config(args.api)
    state = load_state()
    status = new_status()
    interval = int(config.get("poll_interval_secs", DEFAULT_POLL_SECS))
    print("[INFO] bridge running, polling %s every %ds" % (config["api"], interval))
    print("[INFO] allow-list: %s" % config["allowed_peer_id"])
    print("[INFO] status file: %s" % STATUS_PATH)
    while True:
        try:
            drain_once(config, state, status)
        except Exception as exc:  # keep the loop alive; the status file records it
            status["last_error"] = "loop error: %r" % exc
            status["node_reachable"] = False
        write_status(status, state)
        time.sleep(interval)


def cmd_status(args) -> int:
    status = read_json(STATUS_PATH)
    if status is None:
        print("[FAIL] no status file at %s -- the bridge has never run" % STATUS_PATH)
        return 1
    age = (now_ms() - int(status.get("last_loop_at_ms", 0))) / 1000
    print(json.dumps(status, indent=2))
    print()
    if age > 60:
        print("[FAIL] status is %.0fs stale -- the bridge is not running" % age)
        return 1
    if not status.get("node_reachable"):
        print("[FAIL] bridge alive but node unreachable: %s" % status.get("last_error"))
        return 1
    if status.get("acks_pending"):
        print("[WARNING] %d ACK(s) pending delivery" % status["acks_pending"])
    print("[OK] bridge alive, last loop %.0fs ago" % age)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--api", default=None, help="control API base URL")
    sub = parser.add_subparsers(dest="command", required=True)
    learn = sub.add_parser("learn", help="wait for the phone to send, then record it")
    learn.add_argument("--timeout", type=int, default=180)
    learn.add_argument("--write", action="store_true", help="save the result as the allow-list")
    learn.set_defaults(handler=cmd_learn)

    for name, handler in (
        ("discover", cmd_discover),
        ("selftest", cmd_selftest),
        ("once", cmd_once),
        ("run", cmd_run),
        ("status", cmd_status),
    ):
        sub.add_parser(name).set_defaults(handler=handler)
    args = parser.parse_args()
    return args.handler(args)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n[INFO] bridge stopped")
        sys.exit(0)
