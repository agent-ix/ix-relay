//! Recount of `spec/tests.md`: the Test Case Summary rows against the Test
//! Execution Summary totals and GAP-001's row count, refusing a Test Case row
//! (`| TC-...`) inside the traceability tables.

/// The text between byte offsets `from` and `to`, empty when they cross.
fn between(text: &str, from: usize, to: usize) -> &str {
    text.get(from..to).unwrap_or("")
}

/// Splits like Python's `str.splitlines()`: on `\n`, `\r`, `\r\n`, `\v`, `\f`,
/// `\x1c`..`\x1e`, NEL, U+2028 and U+2029, with no empty last line.
fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            continue;
        }
        lines.push(&text[start..at]);
        start = at + c.len_utf8();
        if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
            chars.next();
            start += 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// An unsigned decimal written with ASCII digits, as the text of a number
/// without leading zeros, so any size compares exactly like Python's `int`.
fn ascii_number(text: &str) -> Option<String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let trimmed = text.trim_start_matches('0');
    Some(if trimmed.is_empty() { "0" } else { trimmed }.to_string())
}

/// The M of the first `of M rows have a written test` in `text`, as a number
/// without leading zeros.
fn gap_row_count(text: &str) -> Option<String> {
    const TAIL: &str = " rows have a written test";
    let mut start = 0;
    while let Some(found) = text[start..].find("of ") {
        let digits_at = start + found + 3;
        let digits = text[digits_at..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits > 0 && text[digits_at + digits..].starts_with(TAIL) {
            return ascii_number(&text[digits_at..digits_at + digits]);
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

    for line in splitlines(&text[..tc_at])
        .into_iter()
        .filter(|l| l.starts_with("| TC-"))
    {
        let head: String = line.chars().take(60).collect();
        problems.push(format!(
            "a Test Case Summary row sits in a traceability table: {head}"
        ));
    }

    let rows: Vec<&str> = splitlines(between(text, tc_at, gaps_at))
        .into_iter()
        .filter(|l| l.starts_with("| TC-"))
        .collect();
    // Insertion-ordered, like the summary line it feeds.
    let mut by_type: Vec<(String, usize)> = Vec::new();
    let mut ids: Vec<&str> = Vec::new();
    for row in &rows {
        let cells: Vec<&str> = row.split('|').collect();
        let (Some(id), Some(kind)) = (cells.get(1), cells.get(3)) else {
            let head: String = row.chars().take(60).collect();
            problems.push(format!("malformed Test Case Summary row: {head}"));
            continue;
        };
        let kind = kind.trim().to_string();
        match by_type.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => by_type.push((kind, 1)),
        }
        ids.push(id.trim());
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

    let mut table: Vec<(String, String)> = Vec::new();
    for line in splitlines(&text[exec_at..]) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() <= 3 || cells[1] == "Category" {
            continue;
        }
        let total = match ascii_number(cells[2]) {
            Some(total) => total,
            None if !cells[2].is_empty() && cells[2].chars().all(char::is_numeric) => {
                problems.push(format!(
                    "{}: the total '{}' in the Test Execution Summary is not an ASCII number",
                    cells[1], cells[2]
                ));
                continue;
            }
            None => continue,
        };
        match table.iter_mut().find(|(k, _)| k == cells[1]) {
            Some((_, n)) => *n = total,
            None => table.push((cells[1].to_string(), total)),
        }
    }
    let found_of = |kind: &str| {
        by_type
            .iter()
            .find(|(k, _)| k == kind)
            .map_or(0, |(_, n)| *n)
            .to_string()
    };
    let stated_of = |kind: &str| {
        table
            .iter()
            .find(|(k, _)| k == kind)
            .map_or_else(|| "0".to_string(), |(_, n)| n.clone())
    };
    let mut kinds: Vec<&str> = by_type
        .iter()
        .map(|(k, _)| k.as_str())
        .chain(table.iter().map(|(k, _)| k.as_str()))
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    for kind in kinds {
        let (found, stated) = (found_of(kind), stated_of(kind));
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
        Some(stated) if stated != rows.len().to_string() => problems.push(format!(
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
    use ix_trace_rs::trace;

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
    #[trace("FR-073-AC-1")]
    fn consistent_matrix_prints_the_insertion_ordered_summary() {
        assert_eq!(
            recount(GOOD).as_deref(),
            Ok("test matrix recount: 3 rows, {'Unit': 2, 'Fuzz': 1}")
        );
    }

    #[test]
    #[trace("FR-073-AC-1")]
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
    #[trace("FR-073-AC-1")]
    fn reworded_gap_sentence_is_a_problem() {
        let bad = GOOD.replace("rows have a written test", "rows are fine");
        assert_eq!(
            recount(&bad).unwrap_err(),
            vec!["GAP-001's sentence 'N of M rows have a written test' is missing or reworded"]
        );
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn missing_heading_is_an_error_not_a_panic() {
        assert!(recount("no headings").is_err());
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn a_test_case_row_after_any_python_line_break_is_still_refused() {
        for sep in [
            "\n", "\r", "\r\n", "\u{0b}", "\u{0c}", "\u{1c}", "\u{1d}", "\u{1e}", "\u{85}",
            "\u{2028}", "\u{2029}",
        ] {
            let bad = GOOD.replace(
                "| FR-1 | x |",
                &format!("| FR-1 | x |{sep}| TC-009-01 | s | Unit | FR-1 | x |"),
            );
            let problems = recount(&bad).unwrap_err();
            assert!(
                problems.iter().any(|p| p.contains("| TC-009-01 |")),
                "separator {sep:?}: {problems:?}"
            );
        }
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn splitlines_matches_python() {
        assert_eq!(splitlines("a\r\nb\rc\n"), ["a", "b", "c"]);
        assert_eq!(splitlines("a\n\nb"), ["a", "", "b"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn a_non_ascii_digit_total_is_refused() {
        let bad = GOOD.replace("| Fuzz | 1 |", "| Fuzz | \u{b2} |");
        let problems = recount(&bad).unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("Fuzz: the total '\u{b2}'")),
            "{problems:?}"
        );
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn an_oversized_total_is_a_mismatch_not_a_skipped_row() {
        let huge = "99999999999999999999999";
        let bad = GOOD.replace("| Fuzz | 1 |", &format!("| Fuzz | {huge} |"));
        assert_eq!(
            recount(&bad).unwrap_err(),
            vec![format!(
                "Fuzz: 1 rows in the Test Case Summary, {huge} in the Test Execution Summary"
            )]
        );
        let on_empty_kind = GOOD.replace(
            "| Unit | 2 | 2 |",
            &format!("| Unit | 2 | 2 |\n| Static | {huge} | 0 |"),
        );
        assert_eq!(
            recount(&on_empty_kind).unwrap_err(),
            vec![format!(
                "Static: 0 rows in the Test Case Summary, {huge} in the Test Execution Summary"
            )]
        );
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn leading_zeros_compare_by_value() {
        assert!(recount(&GOOD.replace("| Unit | 2 |", "| Unit | 002 |")).is_ok());
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn a_test_case_row_without_a_type_cell_is_refused() {
        let bad = GOOD.replace("| TC-001-03 | c | Unit | FR-1 | ok |", "| TC-001-03 |");
        let problems = recount(&bad).unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("malformed Test Case Summary row")),
            "{problems:?}"
        );
    }

    #[test]
    #[trace("FR-073-AC-1")]
    fn the_repository_matrix_reconciles() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec/tests.md");
        let text = std::fs::read_to_string(path).unwrap();
        recount(&text).unwrap();
    }
}
