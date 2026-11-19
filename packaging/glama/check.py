"""Starts an MCP server over stdio (the command given) and checks it answers as Glama's
checks expect: initialize, then tools/list with Detour's tools.

    python3 packaging/glama/check.py docker run -i --rm detour-mcp
"""

import json
import subprocess
import sys

p = subprocess.Popen(sys.argv[1:], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)


def send(msg):
    p.stdin.write(json.dumps(msg) + "\n")
    p.stdin.flush()


send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "ci", "version": "0"}}})
info = json.loads(p.stdout.readline())["result"]["serverInfo"]
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
send({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
tools = json.loads(p.stdout.readline())["result"]["tools"]
p.stdin.close()
p.wait(timeout=30)
print(info, len(tools), "tools:", ", ".join(t["name"] for t in tools))
assert len(tools) > 20
