# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Agent-IX
"""ix-relay's documentation figures, rendered by the docs-figures skill.

Regenerate from the repository root with the agent-ix/dev-tools skill:

    python3 -m pip install fonttools
    python3 <dev-tools>/skills/docs-figures/scripts/render.py docs/images/figures.py

Add --check to confirm the committed SVGs are current. Lease durations, the
redelivery limit and the in-flight caps are read from crates/relay-core, the
escalation states from crates/relay-core/src/escalation.rs, and the bus address
from scripts/relay-admin.
"""

import re
from pathlib import Path

from docs_figures import FONTS, SMALL, TITLE, Figure  # noqa: F401

STYLE = "pencil"

ROOT = Path(__file__).resolve().parents[2]

# One role per kind of thing, fixed across the figures.
ROLES = [
    ("code", "lead session in a Herdr tab", "box"),
    ("logic", "relay process on a machine", "box"),
    ("model", "data held in relay-redis", "box"),
    ("evidence", "operator decision", "box"),
    ("ink", "sender, state or transport", "box"),
]


# ---- repository data ---------------------------------------------------------


def _read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def rust_const(path, name):
    """The integer value of `const NAME: ... = EXPR;`, EXPR being products of integers
    optionally wrapped in Duration::from_secs(...)."""
    match = re.search(rf"const {name}\b[^=]*=\s*(.+?);", _read(path), re.S)
    expr = match.group(1).strip()
    expr = re.sub(r"Duration::from_secs\((.*)\)$", r"\1", expr)
    value = 1
    for factor in expr.split("*"):
        value *= int(factor.strip().replace("_", ""))
    return value


def shell_var(name):
    """A plain NAME=value assignment in scripts/relay-admin."""
    return re.search(rf"^{name}=(\S+)", _read("scripts/relay-admin"), re.M).group(1)


def escalation_states():
    """The escalation states, in the order of the enum."""
    body = _read("crates/relay-core/src/escalation.rs")
    enum = body.split("pub enum EscalationStatus {", 1)[1].split("}", 1)[0]
    return [name.lower() for name in re.findall(r"^\s*([A-Z][a-z]+),", enum, re.M)]


INFLIGHT = "crates/relay-core/src/store/inflight.rs"
HOOK_LEASE = rust_const(INFLIGHT, "HOOK_LEASE")
WATCHER_LEASE = rust_const(INFLIGHT, "WATCHER_LEASE")
MAX_REDELIVERIES = rust_const(INFLIGHT, "MAX_REDELIVERIES")
DEAD_LETTER_CAP = rust_const(INFLIGHT, "DEAD_LETTER_CAP")
DIRECTORY_LEASE = rust_const("crates/relay-core/src/store/directory.rs", "DIRECTORY_LEASE")
HUB_ADDR = shell_var("HUB_ADDR").replace("$WG_NET", shell_var("WG_NET"))
BUS_PORT = shell_var("BUS_PORT")


# ---- figures -----------------------------------------------------------------


def topology():
    """README hero: machines with leads and relay processes, one hub store."""
    f = Figure(
        "topology",
        980,
        336,
        "Team leads run in Herdr tabs on each machine. On every machine, agent-msg and "
        "relay-watcher connect over WireGuard to relay-redis on the hub, which holds the "
        "inbox queues, the address directory and the escalations.",
    )
    for i, name in enumerate(["machine A", "machine B", "machine C"]):
        y = 20 + i * 100
        f.box(0, y, 200, 64, "code", name, "leads in Herdr tabs")
        f.box(250, y, 200, 64, "logic", "agent-msg", "relay-watcher")
        f.line([(200, y + 32), (247, y + 32)], "code")
        f.line([(450, y + 32), (470, y + 32)], "logic", arrow=False)
    f.line([(470, 52), (470, 252)], "logic", arrow=False)
    f.line([(470, 152), (647, 152)], "logic")
    f.text(490, 140, "WireGuard", "ink", size=SMALL)
    f.text(490, 176, f"{HUB_ADDR}:{BUS_PORT}", "muted", size=SMALL)
    f.rect(650, 4, 330, 296, "ink", dashed=True, fill=False)
    f.text(670, 30, "hub: relay-redis", "ink", font="title", size=TITLE)
    f.box(675, 52, 280, 62, "model", "inbox queues", "one queue per lead")
    f.box(675, 134, 280, 62, "model", "directory", f"address leases, {DIRECTORY_LEASE} s")
    f.box(675, 216, 280, 62, "evidence", "escalations", " > ".join(escalation_states()))
    f.legend(0, 330, ROLES)
    return f


def delivery():
    """How a message reaches a lead, by the lead's state."""
    f = Figure(
        "delivery",
        980,
        290,
        "A send looks up the recipient in the directory and queues the message in the "
        "recipient's inbox. A working lead gets it from a hook, a starting lead from the "
        "SessionStart hook, an idle lead from relay-watcher, and a blocked lead keeps it "
        "queued while the operator is notified once.",
    )
    f.box(0, 108, 140, 56, "ink", "agent-msg send", "to an address")
    f.box(190, 108, 160, 56, "model", "directory", "address to session")
    f.box(400, 108, 130, 56, "model", "inbox", "queue per session")
    f.line([(140, 136), (187, 136)], "ink")
    f.line([(350, 136), (397, 136)], "model")
    f.note(190, 98, "checks the lease", "muted")
    rows = [
        ("working", "PostToolUse or Stop hook", "mid-turn, in the lead's output", "code"),
        ("starting", "SessionStart hook", "delivers the backlog", "code"),
        ("idle or done", "relay-watcher", "herdr agent prompt", "logic"),
        ("blocked on a dialog", "held in the inbox", "operator notified once", "evidence"),
    ]
    f.line([(530, 136), (570, 136)], "model", arrow=False)
    f.line([(570, 36), (570, 246)], "model", arrow=False)
    for i, (state, by, sub, role) in enumerate(rows):
        y = 8 + i * 70
        f.line([(570, y + 28), (597, y + 28)], "model")
        f.box(600, y, 150, 56, "ink", "lead is", state)
        f.line([(750, y + 28), (797, y + 28)], role)
        f.box(800, y, 180, 56, role, by, sub, dashed=role == "evidence")
    return f


def lifecycle():
    """The lease, acknowledgement, redelivery and dead-letter path of one batch."""
    f = Figure(
        "lifecycle",
        980,
        290,
        f"A popped batch moves from the inbox to in flight under a lease of {HOOK_LEASE} s for "
        f"a hook or {WATCHER_LEASE} s for the watcher. An acknowledgement ends it. An expired "
        f"lease returns the batch to the inbox, up to {MAX_REDELIVERIES} times, and the next "
        "expiry moves it to the dead-letter list.",
    )
    f.box(0, 90, 170, 62, "model", "inbox", "pending, in order")
    f.box(330, 90, 200, 62, "logic", "in flight", f"lease: hook {HOOK_LEASE} s, watcher {WATCHER_LEASE} s")
    f.box(690, 90, 170, 62, "evidence", "acknowledged", "removed from in flight")
    f.box(330, 214, 200, 62, "ink", "dead letters", f"newest {DEAD_LETTER_CAP:,} kept")
    f.line([(170, 121), (327, 121)], "model")
    f.note(190, 111, "pop", "muted")
    f.line([(530, 121), (687, 121)], "logic", weight=2.4)
    f.note(550, 111, "acknowledge", "muted")
    f.line([(430, 90), (430, 36), (85, 36), (85, 87)], "logic", dashed=True)
    f.note(110, 28, f"lease expires: back to the inbox, marked redelivered, up to {MAX_REDELIVERIES} times", "muted")
    f.line([(430, 152), (430, 211)], "ink", dashed=True)
    f.note(446, 186, f"expires after {MAX_REDELIVERIES} redeliveries", "muted")
    return f


def escalation():
    """An escalation from filing to resolution."""
    states = escalation_states()
    f = Figure(
        "escalation",
        980,
        250,
        "A lead files an escalation and keeps working. The operator lists it, investigates it in "
        "a Claude Code session, and resolves it. The decision goes back to the lead over the bus "
        "and is posted as a comment on the Linear ticket.",
    )
    f.box(0, 70, 160, 62, "code", "lead", "agent-msg escalate")
    f.box(240, 70, 170, 62, "model", states[0], "stored in relay-redis")
    f.box(490, 70, 180, 62, "evidence", states[1], "session in the lead's cwd")
    f.box(750, 70, 130, 62, "evidence", states[2], "decision recorded")
    f.line([(160, 101), (237, 101)], "code")
    f.line([(410, 101), (487, 101)], "model")
    f.line([(670, 101), (747, 101)], "evidence")
    f.note(176, 91, "file", "muted")
    f.note(418, 91, "investigate", "muted")
    f.note(692, 91, "resolve", "muted")
    f.box(0, 170, 170, 56, "ink", "lead keeps working", "other tasks meanwhile", dashed=True)
    f.line([(80, 132), (80, 167)], "code", dashed=True)
    f.line([(815, 132), (815, 150)], "evidence", arrow=False)
    f.line([(690, 150), (865, 150)], "evidence", arrow=False)
    f.line([(690, 150), (690, 167)], "evidence")
    f.line([(865, 150), (865, 167)], "evidence")
    f.box(620, 170, 140, 56, "code", "the lead", "gets the decision")
    f.box(790, 170, 150, 56, "ink", "Linear ticket", "decision comment")
    return f


FIGURES = [topology, delivery, lifecycle, escalation]
