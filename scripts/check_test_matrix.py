#!/usr/bin/env python3
"""Recount spec/tests.md: the Test Case Summary rows against the Test Execution
Summary totals and GAP-001's row count, and refuse a Test Case row (`| TC-...`)
inside the traceability tables. The recount procedure of AGE-2027's review."""
import re
import sys
from collections import Counter

text = open("spec/tests.md", encoding="utf-8").read()
tc_at = text.index("## Test Case Summary")
gaps_at = text.index("## Coverage Gaps")
exec_at = text.index("## Test Execution Summary")
problems = []

# Rows in the traceability tables must be requirement rows, not test-case rows.
for line in text[:tc_at].splitlines():
    if line.startswith("| TC-"):
        problems.append(f"a Test Case Summary row sits in a traceability table: {line[:60]}")

rows = [l for l in text[tc_at:gaps_at].splitlines() if l.startswith("| TC-")]
by_type = Counter(l.split("|")[3].strip() for l in rows)
ids = [l.split("|")[1].strip() for l in rows]
for dup in sorted({i for i in ids if ids.count(i) > 1}):
    problems.append(f"duplicate test id {dup}")

table = {}
for l in text[exec_at:].splitlines():
    cells = [c.strip() for c in l.split("|")]
    if len(cells) > 3 and cells[2].isdigit() and cells[1] != "Category":
        table[cells[1]] = int(cells[2])
for kind in set(by_type) | set(table):
    if by_type.get(kind, 0) != table.get(kind, 0):
        problems.append(
            f"{kind}: {by_type.get(kind, 0)} rows in the Test Case Summary, "
            f"{table.get(kind, 0)} in the Test Execution Summary"
        )
m = re.search(r"of (\d+) rows have a written test", text[gaps_at:exec_at])
if not m:
    problems.append("GAP-001's sentence 'N of M rows have a written test' is missing or reworded")
elif int(m.group(1)) != len(rows):
    problems.append(f"GAP-001 says {m.group(1)} rows, the Test Case Summary has {len(rows)}")

if problems:
    print("\n".join(sorted(problems)))
    sys.exit(1)
print(f"test matrix recount: {len(rows)} rows, {dict(by_type)}")
