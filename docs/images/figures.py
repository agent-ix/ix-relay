# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Agent-IX
"""ix-relay's documentation figures, rendered by the docs-figures skill.

Regenerate from the repository root with the agent-ix/dev-tools skill:

    python3 -m pip install fonttools
    python3 <dev-tools>/skills/docs-figures/scripts/render.py docs/images/figures.py

Add --check to confirm the committed SVGs are current. Only topology and escalation are
defined here: both print the escalation states lower-cased from the enum names in
crates/relay-core/src/escalation.rs, which docs/images/figures.json cannot yet express
(no text transforms in value bindings). The delivery and lifecycle figures are in
docs/images/figures.json and rendered by `make figures`.
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


FIGURES = [topology, escalation]
