#!/usr/bin/env python3
"""Drive one destructive-action scenario through the real TUI and check the cluster.

ACT-01..08 in docs/QA-TESTPLAN.md were never executed: they mutate a cluster, so a QA pass
that runs unattended must not touch a live one. This drives the actual keybinding, confirm
popup and dispatch path against a throwaway namespace, so the paths a user reaches in admin
mode are exercised rather than assumed.

Usage:  scripts/act-drive.py <scenario> [namespace] [deployment]
        scenario: scale | stop | restart | delete-pod | stale-target | ui-latency

One scenario per invocation, on purpose — each run leaves the cluster inspectable, and a
failure points at one action instead of a batch.
"""
import fcntl
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import time

NS = sys.argv[2] if len(sys.argv) > 2 else "lmd-qa"
DEP = sys.argv[3] if len(sys.argv) > 3 else "qa-dummy"
BIN = os.path.expanduser("~/.cargo/bin/lmd-top")


class KubectlFailed(RuntimeError):
    pass


def kc(*args, timeout=30, allow_fail=False):
    """Run kubectl and return stdout, raising unless the caller expects a failure.

    Swallowing failures here left a node cordoned once: a transient query error returned an
    empty string, the cleanup read that as "nothing was cordoned", and iterated over nothing.
    A test that mutates a cluster must not treat "I could not tell" as "there is nothing to
    undo", so failures are loud by default and opted out of explicitly.
    """
    r = subprocess.run(["kubectl", *args], capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0 and not allow_fail:
        raise KubectlFailed(f"kubectl {' '.join(args)} -> {r.returncode}: {r.stderr.strip()}")
    return r.stdout.strip()


def replicas():
    # The deployment is deliberately deleted in the stale-target scenario, so absence is a
    # valid answer here rather than an error.
    return kc("get", "deploy", DEP, "-n", NS, "-o", "jsonpath={.spec.replicas}",
              allow_fail=True)


def pods():
    out = kc("get", "pods", "-n", NS, "-l", f"app={DEP}", "-o",
             "jsonpath={range .items[*]}{.metadata.name}{'\\n'}{end}")
    return sorted(p for p in out.splitlines() if p.strip())


def wait_until(pred, timeout=90, poll=2):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(poll)
    return False


class Tui:
    """A real pty so crossterm sees a terminal and the render loop runs."""

    def __init__(self, mode="admin", cols=150, rows=42):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        env = {**os.environ, "TERM": "xterm-256color", "LMD_NS": NS, "LMD_THEME": "default"}
        env.pop("NO_COLOR", None)
        self.p = subprocess.Popen([BIN, "--mode", mode], stdin=slave, stdout=slave,
                                  stderr=slave, env=env, start_new_session=True)
        os.close(slave)
        self.buf = ""

    def drain(self, secs):
        end = time.monotonic() + secs
        while time.monotonic() < end:
            if select.select([self.master], [], [], 0.1)[0]:
                try:
                    self.buf += os.read(self.master, 262144).decode("utf-8", "replace")
                except OSError:
                    return

    def send(self, keys, settle=1.5):
        os.write(self.master, keys.encode() if isinstance(keys, str) else keys)
        self.drain(settle)

    def screen(self):
        return re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][A-Z0-9]", "", self.buf)

    def saw(self, needle):
        return needle.lower() in self.screen().lower()

    def close(self):
        """Quit without confirming anything that happens to be open.

        This used to send `q` then `y` unconditionally. With an apply confirm still on screen
        that `y` *approved the apply*, so shutting the harness down created a compile Job in
        the cluster. Escape out of any overlay first, and only answer the exit prompt.
        """
        try:
            for _ in range(4):
                os.write(self.master, b"\x1b")   # dismiss overlays, one layer at a time
                self.drain(0.3)
            self.buf = ""
            os.write(self.master, b"q")
            self.drain(1.0)
            if "quit" in self.screen().lower() or "exit" in self.screen().lower():
                os.write(self.master, b"y")
                self.drain(0.5)
        except OSError:
            pass
        try:
            self.p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.p.terminate()
            self.p.wait()
        os.close(self.master)


def report(scenario, ok, detail):
    print(f"{'PASS' if ok else 'FAIL'}  {scenario}  {detail}", flush=True)
    return 0 if ok else 1


def overview(t):
    """Land on Overview with the deployment selected. Section 0 is Overview."""
    t.drain(9)  # first full collect
    t.send("0", 1.0)


def scenario_scale(t):
    """ACT-01: s toggles replicas, twice, with the view still coherent afterwards."""
    overview(t)
    before = replicas()
    t.send("s", 1.5)
    prompted = t.saw("scale")
    t.send("y", 3.0)
    down = wait_until(lambda: replicas() == "0", 60)
    if not (down and prompted):
        return report("ACT-01", False,
                      f"1->0 failed: replicas={replicas()} (was {before}) prompt={prompted}")
    t.send("s", 1.5)
    t.send("y", 3.0)
    up = wait_until(lambda: replicas() == "1", 90)
    t.drain(8)
    return report("ACT-01", up and t.saw(DEP),
                  f"1->0->1 ok (replicas={replicas()}), view still lists {DEP}")


def scenario_stop(t):
    """ACT-02: x stops serving; the action is only offered while running."""
    wait_until(lambda: replicas() == "1", 90)
    overview(t)
    t.send("x", 1.5)
    prompted = t.saw("stop")
    t.send("y", 3.0)
    stopped = wait_until(lambda: replicas() == "0", 60)
    ok = stopped and prompted
    # Restore so later scenarios start from a known state.
    kc("scale", "deploy", DEP, "-n", NS, "--replicas=1")
    wait_until(lambda: replicas() == "1", 90)
    return report("ACT-02", ok, f"stop -> replicas 0 ({stopped}), prompt={prompted}")


def scenario_restart(t):
    """ACT-03: S rolls the pods, replacing them."""
    wait_until(lambda: replicas() == "1" and pods(), 90)
    before = pods()
    overview(t)
    t.send("S", 1.5)
    prompted = t.saw("restart")
    t.send("y", 4.0)
    replaced = wait_until(lambda: pods() and pods() != before, 120)
    return report("ACT-03", replaced and prompted,
                  f"pods {before} -> {pods()}, prompt={prompted}")


def goto_pods(t):
    """Serving section, then two sub-tabs across: Serving -> Perf -> Pods."""
    t.drain(9)
    t.send("2", 1.2)
    t.send("]", 1.2)
    t.send("]", 1.8)


def scenario_delete_pod(t):
    """ACT-06: Delete a pod and watch the ReplicaSet replace it.

    Delete has no direct key on purpose — it is reachable only through the action menu, so a
    danger-tier action cannot happen from one stray keystroke. The first version of this test
    pressed `D` on the view and saw nothing happen, which was the product being right.
    """
    wait_until(lambda: pods(), 90)
    before = pods()
    goto_pods(t)
    t.buf = ""
    t.send("a", 1.5)                     # action menu for the selected pod
    menu = t.saw("actions ·") or t.saw("delete")
    t.send("D", 1.5)                     # menu accelerator
    prompted = t.saw("delete")
    t.send("y", 4.0)
    replaced = wait_until(lambda: pods() and pods() != before, 120)
    return report("ACT-06", replaced and prompted and menu,
                  f"pods {before} -> {pods()}, menu={menu}, prompt={prompted}")


def scenario_delete_pod_gated(t):
    """PERM: Delete must be refused in admin mode, naming the mode it needs."""
    wait_until(lambda: pods(), 90)
    before = pods()
    goto_pods(t)
    t.buf = ""
    t.send("a", 1.5)
    t.send("D", 2.5)
    refused = t.saw("danger")
    unchanged = pods() == before
    return report("ACT-06-gate", refused and unchanged,
                  f"refused in admin={refused} (names danger), pods unchanged={unchanged}")


def scenario_stale_target(t):
    """ACT-07: act on something deleted underneath us — a clear error, no wedged state."""
    overview(t)
    t.send("s", 1.5)          # open the confirm, then delete the target from outside
    kc("delete", "deploy", DEP, "-n", NS, "--wait=false")
    wait_until(lambda: replicas() == "", 60)
    t.send("y", 6.0)
    said_failed = t.saw("failed") or t.saw("not found") or t.saw("notfound")
    alive = t.p.poll() is None
    # Put the fixture back for whatever runs next.
    subprocess.run(["kubectl", "apply", "-f", "/tmp/qa-ns.yaml"], capture_output=True, timeout=60)
    return report("ACT-07", said_failed and alive,
                  f"reported the failure={said_failed}, TUI still running={alive}")


def jobs():
    out = kc("get", "jobs", "-n", NS, "-o",
             "jsonpath={range .items[*]}{.metadata.name} {end}", allow_fail=True)
    return sorted(out.split())


def scenario_dry_run_apply(t):
    """ACT-04: v validates against the API server and changes nothing; y then applies.

    The whole point of the separation is that an operator can prove a manifest is acceptable
    before creating anything, so the assertion that matters is that `v` leaves the namespace
    with no new Job.
    """
    before = jobs()
    t.drain(10)
    t.send("4", 2.0)                 # Deploy section, Library landing
    t.send("j", 1.2)
    t.send("j", 1.5)                 # onto a compilable row
    t.buf = ""
    t.send("c", 2.5)                 # compile form
    form = t.saw("compile ·")
    # Enter opens a placement picker first (which node to compile on); its first row is
    # "any". A second Enter turns the form plus placement into the manifest and the confirm.
    t.send("\r", 2.5)
    picked = t.saw("compile node")
    t.buf = ""
    t.send("\r", 3.0)
    confirm = t.saw("apply manifest") or t.saw("run this operation")
    t.buf = ""
    t.send("v", 7.0)                 # server-side dry run (a synchronous kubectl call)
    screen = t.screen().lower()
    # "invalid" contains "valid", so match the marker the success path prints.
    validated = ("valid ✓" in screen) or ("valid " in screen and "invalid" not in screen)
    after_v = jobs()
    if not (form and picked and confirm and validated and after_v == before):
        return report("ACT-04", False,
                      f"form={form} picker={picked} confirm={confirm} "
                      f"validated={validated} jobs {before} -> {after_v} "
                      f"(v must not create anything)")
    t.buf = ""
    t.send("y", 6.0)                 # now actually apply
    created = wait_until(lambda: len(jobs()) > len(before), 90)
    made = [j for j in jobs() if j not in before]
    # Clean up whatever we just created; this namespace is a fixture, not a workload.
    for j in made:
        subprocess.run(["kubectl", "delete", "job", j, "-n", NS, "--wait=false"],
                       capture_output=True, timeout=30)
    return report("ACT-04", created,
                  f"v validated with no change, then apply created {made}")


def cordoned_nodes():
    # Space-separated, deliberately escape-free. A newline escape inside a jsonpath is one
    # backslash away from becoming a real newline in this file, and kubectl then fails with
    # "unterminated quoted string" — which is how this list silently emptied once and left a
    # node cordoned after cleanup iterated over nothing.
    out = kc("get", "nodes", "-o",
             "jsonpath={range .items[?(@.spec.unschedulable==true)]}{.metadata.name} {end}")
    return sorted(out.split())


def scenario_cordon(t):
    """ACT-05: cordon a node from the Nodes view, then uncordon it.

    Cordon only blocks *new* scheduling — running pods stay put — and this uncordons in a
    finally, including on failure. Any node left unschedulable by a crashed run would be a
    cluster-wide problem, so the restore is unconditional.
    """
    already = cordoned_nodes()
    node = None
    try:
        t.drain(9)
        t.send("3", 1.5)               # Infra section, Nodes landing
        t.buf = ""
        t.send("a", 1.5)               # Cordon lives in the action menu, not on a bare key
        t.send("C", 1.5)
        # Read the target out of the confirm prompt ("cordon ... node <name>?") rather than by
        # diffing a query. Cleanup then works even if a kubectl call fails, and it also avoids
        # matching the word "cordon" inside the *Uncordon* label on an already-cordoned node.
        m = re.search(r"cordon[^\n]*?node\s+(\S+?)\?", t.screen(), re.I)
        node = m.group(1) if m else None
        prompted = node is not None
        t.send("y", 4.0)
        ok_down = prompted and wait_until(lambda: node in cordoned_nodes(), 60)
        if not ok_down:
            return report("ACT-05", False,
                          f"cordon did not take: prompt={prompted} node={node} "
                          f"cordoned={cordoned_nodes()}")
        t.drain(6)
        t.buf = ""
        t.send("a", 1.5)
        t.send("u", 1.5)               # Uncordon, same menu
        t.send("y", 4.0)
        ok_up = wait_until(lambda: node not in cordoned_nodes(), 60)
        return report("ACT-05", ok_up,
                      f"cordoned {node} then uncordoned it (restored={ok_up})")
    finally:
        # Unconditional, and belt-and-braces: uncordon the node we saw ourselves cordon, then
        # sweep for anything else newly unschedulable. If the sweep's query fails we say so
        # loudly instead of exiting quietly — "I could not tell" is not "nothing to undo".
        if node:
            subprocess.run(["kubectl", "uncordon", node], capture_output=True, timeout=30)
        try:
            for n in cordoned_nodes():
                if n not in already:
                    subprocess.run(["kubectl", "uncordon", n], capture_output=True, timeout=30)
        except (KubectlFailed, subprocess.SubprocessError) as exc:
            print(f"!! CLEANUP COULD NOT VERIFY node state: {exc}\n"
                  f"!! check `kubectl get nodes` for anything left cordoned", flush=True)


def scenario_ui_latency(t):
    """ACT-08: a keypress must produce a frame well inside the freeze budget."""
    overview(t)
    t.buf = ""
    t0 = time.monotonic()
    os.write(t.master, b"s")
    first = None
    while time.monotonic() - t0 < 12:
        if select.select([t.master], [], [], 0.05)[0]:
            try:
                t.buf += os.read(t.master, 262144).decode("utf-8", "replace")
            except OSError:
                break
            if t.buf.strip():
                first = time.monotonic() - t0
                break
    t.send("n", 2.0)  # cancel the confirm we opened
    return report("ACT-08", first is not None and first < 8.0,
                  f"first frame {first if first is None else round(first, 3)}s (budget 8s)")


# (function, permission mode). Delete is danger-gated on purpose — running it in admin is a
# test of the gate, not of the action, and the first attempt here failed for exactly that
# reason.
SCENARIOS = {
    "scale": (scenario_scale, "admin"),
    "stop": (scenario_stop, "admin"),
    "restart": (scenario_restart, "admin"),
    "delete-pod": (scenario_delete_pod, "danger"),
    "delete-pod-gated": (scenario_delete_pod_gated, "admin"),
    "cordon": (scenario_cordon, "admin"),
    "dry-run-apply": (scenario_dry_run_apply, "admin"),
    "stale-target": (scenario_stale_target, "admin"),
    "ui-latency": (scenario_ui_latency, "admin"),
}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else ""
    if which not in SCENARIOS:
        sys.exit(f"usage: {sys.argv[0]} <{'|'.join(SCENARIOS)}> [ns] [deployment]")
    fn, mode = SCENARIOS[which]
    tui = Tui(mode=mode)
    try:
        code = fn(tui)
    finally:
        tui.close()
    sys.exit(code)
