#!/usr/bin/env python3
"""Deterministic capability guard derived solely from the v2 manifest."""

import argparse
import json
from pathlib import PurePosixPath

from orchestration_contract import (
    ContractError, load_manifest, protected_paths, requires_delivery_review,
    valid_transition,
)


def _normal(path):
    return str(PurePosixPath(path.replace("\\", "/")))


def may_write(manifest, role, path, packet_files=None):
    if role == "OPERATOR":
        return True
    if role not in manifest["semantic_roles"]:
        return False
    normal = _normal(path)
    if role in ("IMPLEMENTER", "PLATFORM_IMPLEMENTER"):
        packet_files = {_normal(item) for item in (packet_files or [])}
        return normal != "." and not normal.startswith("../") and normal in packet_files
    return any(normal.startswith(scope) for scope in manifest["semantic_roles"][role].get("writes", []))


def evaluate(manifest, role, action, path=None, files=None, current=None, target=None, description="", reviews_complete=False, packet_files=None):
    files = files or []
    if role not in manifest["semantic_roles"]:
        return False, "unknown semantic role"
    if action == "write":
        return (may_write(manifest, role, path or "", packet_files), "path is outside role writable scope or packet scope")
    if action == "transition":
        return (valid_transition(manifest, current, target), "lifecycle transition is not permitted")
    if action == "isolation":
        required = manifest["semantic_roles"][role].get("isolation_required", False)
        return (required, "role does not require writer isolation")
    if action == "review":
        protected = protected_paths(manifest, files)
        required = bool(protected) or requires_delivery_review(manifest, files, description)
        return required, "independent review is not required for this scope"
    if action == "integrate":
        protected = protected_paths(manifest, files)
        required = bool(protected) or requires_delivery_review(manifest, files, description)
        return ((not required or reviews_complete), "required independent review is outstanding")
    return False, "unknown action"


def main():
    parser = argparse.ArgumentParser(description="Query Orchestration Control Plane v2 permissions")
    parser.add_argument("--role", required=True)
    parser.add_argument("--action", required=True, choices=["write", "transition", "isolation", "review", "integrate"])
    parser.add_argument("--path")
    parser.add_argument("--files", nargs="*", default=[])
    parser.add_argument("--packet-files", nargs="*", default=[], help="Exact writable paths authorized by the worker packet")
    parser.add_argument("--from-state", dest="current")
    parser.add_argument("--to-state", dest="target")
    parser.add_argument("--description", default="")
    parser.add_argument("--reviews-complete", action="store_true")
    args = parser.parse_args()
    try:
        manifest = load_manifest()
    except ContractError as exc:
        print(json.dumps({"allowed": False, "reason": str(exc)}))
        return 2
    allowed, reason = evaluate(manifest, args.role, args.action, args.path, args.files, args.current, args.target, args.description, args.reviews_complete, args.packet_files)
    print(json.dumps({"allowed": allowed, "reason": "ok" if allowed else reason}))
    return 0 if allowed else 1


if __name__ == "__main__":
    raise SystemExit(main())
