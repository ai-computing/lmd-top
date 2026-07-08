#!/usr/bin/env python3
"""Inject the endpoint-score-observer picker into an existing EPP plugins config.

Reads an EndpointPickerConfig YAML (stdin), returns it (stdout) with:
  - `endpoint-score-observer` added to the top-level `plugins` list, and
  - each schedulingProfile's picker replaced by it (our picker delegates to max-score,
    so selection behavior is preserved).

Idempotent: running twice is a no-op. Only PyYAML is required.
"""
import sys
import yaml

PLUGIN_TYPE = "endpoint-score-observer"
# Picker plugin types we replace (a profile may reference one explicitly, or rely on the
# implicit default max-score-picker — in which case we simply become the picker).
KNOWN_PICKERS = {"max-score-picker", "random-picker", "weighted-random-picker"}


def main() -> int:
    cfg = yaml.safe_load(sys.stdin.read())
    if not isinstance(cfg, dict):
        sys.stderr.write("input is not a valid EndpointPickerConfig\n")
        return 1

    plugins = cfg.setdefault("plugins", [])
    have = any((p or {}).get("type") == PLUGIN_TYPE for p in plugins)
    if not have:
        plugins.append({"type": PLUGIN_TYPE})

    # name -> type, to find which profile refs are pickers.
    name_to_type = {}
    for p in plugins:
        p = p or {}
        name_to_type[p.get("name", p.get("type"))] = p.get("type")

    for prof in cfg.get("schedulingProfiles", []) or []:
        refs = prof.get("plugins", []) or []
        # drop any existing picker refs
        refs = [r for r in refs if name_to_type.get((r or {}).get("pluginRef")) not in KNOWN_PICKERS]
        # add ours as the picker if not already present
        if not any((r or {}).get("pluginRef") == PLUGIN_TYPE for r in refs):
            refs.append({"pluginRef": PLUGIN_TYPE})
        prof["plugins"] = refs

    yaml.safe_dump(cfg, sys.stdout, default_flow_style=False, sort_keys=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
