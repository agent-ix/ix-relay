//! Recount of `spec/tests.md`: the Test Case Summary rows against the Test
//! Execution Summary totals and GAP-001's row count, refusing a Test Case row
//! (`| TC-...`) inside the traceability tables.

/// The text between byte offsets `from` and `to`, empty when they cross.
fn between(text: &str, from: usize, to: usize) -> &str {
    text.get(from..to).unwrap_or("")
}

/// The number M of the first `of M rows have a written test` in `text`.
fn gap_row_count(text: &str) -> Option<usize> {
    const TAIL: &str = " rows have a written test";
    let mut start = 0;
    while let Some(found) = text[start..].find("of ") {
        let digits_at = start + found + 3;
        let digits = text[digits_at..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits > 0 && text[digits_at + digits..].starts_with(TAIL) {
            return text[digits_at..digits_at + digits].parse().ok();
        }
        start = digits_at;
    }
    None
}

/// Recounts the matrix. `Ok` is the one-line summary; `Err` is every problem,
/// sorted.
pub fn recount(text: &str) -> Result<String, Vec<String>> {
    let marker = |heading: &str| {
        text.find(heading)
            .ok_or_else(|| vec![format!("spec/tests.md has no '{heading}' heading")])
    };
    let tc_at = marker("## Test Case Summary")?;
    let gaps_at = marker("## Coverage Gaps")?;
    let exec_at = marker("## Test Execution Summary")?;
    let mut problems = Vec::new();

    for line in text[..tc_at].lines().filter(|l| l.starts_with("| TC-")) {
        let head: String = line.chars().take(60).collect();
        problems.push(format!(
            "a Test Case Summary row sits in a traceability table: {head}"
        ));
    }

    let rows: Vec<&str> = between(text, tc_at, gaps_at)
        .lines()
        .filter(|l| l.starts_with("| TC-"))
        .collect();
    // Insertion-ordered, like the summary line it feeds.
    let mut by_type: Vec<(String, usize)> = Vec::new();
    let mut ids: Vec<&str> = Vec::new();
    for row in &rows {
        let cells: Vec<&str> = row.split('|').collect();
        let kind = cells.get(3).map_or("", |c| c.trim()).to_string();
        match by_type.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => by_type.push((kind, 1)),
        }
        ids.push(cells.get(1).map_or("", |c| c.trim()));
    }
    let mut dups: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| ids.iter().filter(|other| *other == id).count() > 1)
        .collect();
    dups.sort_unstable();
    dups.dedup();
    for dup in dups {
        problems.push(format!("duplicate test id {dup}"));
    }

    let mut table: Vec<(String, usize)> = Vec::new();
    for line in text[exec_at..].lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() > 3
            && !cells[2].is_empty()
            && cells[2].bytes().all(|b| b.is_ascii_digit())
            && cells[1] != "Category"
            && let Ok(total) = cells[2].parse::<usize>()
        {
            match table.iter_mut().find(|(k, _)| k == cells[1]) {
                Some((_, n)) => *n = total,
                None => table.push((cells[1].to_string(), total)),
            }
        }
    }
    let count = |list: &[(String, usize)], kind: &str| {
        list.iter().find(|(k, _)| k == kind).map_or(0, |(_, n)| *n)
    };
    let mut kinds: Vec<&str> = by_type
        .iter()
        .chain(&table)
        .map(|(k, _)| k.as_str())
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    for kind in kinds {
        let (found, stated) = (count(&by_type, kind), count(&table, kind));
        if found != stated {
            problems.push(format!(
                "{kind}: {found} rows in the Test Case Summary, {stated} in the Test Execution Summary"
            ));
        }
    }

    match gap_row_count(between(text, gaps_at, exec_at)) {
        None => problems.push(
            "GAP-001's sentence 'N of M rows have a written test' is missing or reworded".into(),
        ),
        Some(stated) if stated != rows.len() => problems.push(format!(
            "GAP-001 says {stated} rows, the Test Case Summary has {}",
            rows.len()
        )),
        Some(_) => {}
    }

    if !problems.is_empty() {
        problems.sort();
        return Err(problems);
    }
    let kinds: Vec<String> = by_type.iter().map(|(k, n)| format!("'{k}': {n}")).collect();
    Ok(format!(
        "test matrix recount: {} rows, {{{}}}",
        rows.len(),
        kinds.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "\
## Requirements Traceability

| FR-1 | x |

## Test Case Summary

| TC-001-01 | a | Unit | FR-1 | ok |
| TC-001-02 | b | Fuzz | FR-1 | ok |
| TC-001-03 | c | Unit | FR-1 | ok |

## Coverage Gaps

| GAP-001 | 2 of 3 rows have a written test; more. |

## Test Execution Summary

| Category | Total | Named |
|----------|-------|-------|
| Fuzz | 1 | 0 |
| Unit | 2 | 2 |
";

    #[test]
    fn consistent_matrix_prints_the_insertion_ordered_summary() {
        assert_eq!(
            recount(GOOD).as_deref(),
            Ok("test matrix recount: 3 rows, {'Unit': 2, 'Fuzz': 1}")
        );
    }

    #[test]
    fn every_problem_is_reported_sorted() {
        let bad = GOOD
            .replace("2 of 3 rows", "2 of 4 rows")
            .replace("| Unit | 2 |", "| Unit | 3 |")
            .replace("| TC-001-03 | c |", "| TC-001-01 | c |")
            .replace(
                "| FR-1 | x |",
                "| FR-1 | x |\n| TC-009-01 | s | Unit | FR-1 | x |",
            );
        let problems = recount(&bad).unwrap_err();
        assert_eq!(
            problems,
            vec![
                "GAP-001 says 4 rows, the Test Case Summary has 3".to_string(),
                "Unit: 2 rows in the Test Case Summary, 3 in the Test Execution Summary".into(),
                "a Test Case Summary row sits in a traceability table: | TC-009-01 | s | Unit | FR-1 | x |".into(),
                "duplicate test id TC-001-01".into(),
            ]
        );
    }

    #[test]
    fn reworded_gap_sentence_is_a_problem() {
        let bad = GOOD.replace("rows have a written test", "rows are fine");
        assert_eq!(
            recount(&bad).unwrap_err(),
            vec!["GAP-001's sentence 'N of M rows have a written test' is missing or reworded"]
        );
    }

    #[test]
    fn missing_heading_is_an_error_not_a_panic() {
        assert!(recount("no headings").is_err());
    }

    #[test]
    fn the_repository_matrix_reconciles() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec/tests.md");
        let text = std::fs::read_to_string(path).unwrap();
        recount(&text).unwrap();
    }
}
