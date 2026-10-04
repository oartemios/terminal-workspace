#!/usr/bin/python3
"""Deterministic protocol peer for runtime failure/permission tests; no network."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

descriptor = json.loads(Path("plugin.json").read_text())["plugin"]
for line in sys.stdin:
    request = json.loads(line)
    op = request["op"]
    result = None
    error = None
    if op == "describe":
        result = dict(descriptor)
        if "mismatch" in sys.argv:
            result["name"] = "Unexpected worker"
    elif op == "view":
        result = {"view": {"title": "Probe", "location": "", "items": [
            {"id": "probe.item", "title": "Probe item", "kind": "probe"}],
            "parent": None, "command_defaults": []}, "icons": {"probe.item": "•"}}
    elif op == "actions":
        result = [{"label": "Foreign action", "command_id": "core.plugin.disable",
                   "invocation": {"id": "core.plugin.disable", "item": None, "args": ["files"]},
                   "is_default": True}]
    else:
        command = request["invocation"]["id"].split(".")[-1]
        if command == "crash":
            os._exit(23)
        if command == "hang":
            time.sleep(30)
        if command == "malformed":
            print("not JSON", flush=True)
            continue
        if command == "oversized":
            print("x" * 1_048_577, flush=True)
            continue
        if command == "wrong_id":
            request["request_id"] += 1
        if command == "error":
            error = "Expected domain error"
        else:
            context = request["context"]
            content = {"pid": os.getpid(), "root": context["root"], "settings": context["settings"],
                       "environment": {name: os.environ.get(name) for name in [
                           "TW_RUNTIME_TEST_ENV", "TW_RUNTIME_TEST_CREDENTIAL", "TW_UNDECLARED_SECRET", "HOME"]}}
            if command == "child":
                content["child"] = subprocess.Popen(["/bin/sleep", "30"]).pid
            result = {"Output": {"source": "Probe", "status": "ok", "content": json.dumps(content)}}
    reply = {"protocol": 1, "request_id": request["request_id"],
             "result": {"Err": error} if error else {"Ok": result}}
    print(json.dumps(reply), flush=True)
