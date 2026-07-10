use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use super::csv_key_diff::{self, CsvData, KeyValue};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffSelection {
    key_columns: Vec<String>,
    rows: Vec<RowSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RowSelector {
    key: KeyValue,
    occurrence: usize,
}

pub fn extract_diff_rows_to_files(
    left_path: &Path,
    right_path: &Path,
    diff_path: &Path,
    output_left_path: &Path,
    output_right_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let left_csv = csv_key_diff::read_csv(left_path)?;
    let right_csv = csv_key_diff::read_csv(right_path)?;
    let diff_text = fs::read_to_string(diff_path)?;
    let selection = parse_diff_selection(&diff_text)?;
    validate_selection_columns(&left_csv, &selection, "left")?;
    validate_selection_columns(&right_csv, &selection, "right")?;

    let left_rows = extract_rows(&left_csv, &selection);
    let right_rows = extract_rows(&right_csv, &selection);

    write_csv(output_left_path, &left_csv.headers, &left_rows)?;
    write_csv(output_right_path, &right_csv.headers, &right_rows)?;

    Ok(())
}

fn validate_selection_columns(
    csv: &CsvData,
    selection: &DiffSelection,
    side: &str,
) -> Result<(), Box<dyn Error>> {
    for column in &selection.key_columns {
        if !csv.headers.iter().any(|header| header == column) {
            return Err(format!("{side} CSV is missing diff key column: {column}").into());
        }
    }
    Ok(())
}

fn extract_rows(csv: &CsvData, selection: &DiffSelection) -> Vec<Vec<String>> {
    let header_index = header_index(&csv.headers);
    let mut occurrences_by_key = HashMap::<KeyValue, HashSet<usize>>::new();

    for selector in &selection.rows {
        occurrences_by_key
            .entry(selector.key.clone())
            .or_default()
            .insert(selector.occurrence);
    }

    let mut seen_occurrences = HashMap::<KeyValue, usize>::new();
    let mut extracted = Vec::new();

    for row in &csv.rows {
        let key = extract_key_from_row(row, &header_index, &selection.key_columns);
        let occurrence = seen_occurrences.entry(key.clone()).or_insert(0);

        if occurrences_by_key
            .get(&key)
            .is_some_and(|occurrences| occurrences.contains(occurrence))
        {
            extracted.push(row.clone());
        }

        *occurrence += 1;
    }

    extracted
}

fn header_index(headers: &[String]) -> HashMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(idx, name)| (name.clone(), idx))
        .collect()
}

fn extract_key_from_row(
    row: &[String],
    index: &HashMap<String, usize>,
    key_columns: &[String],
) -> KeyValue {
    KeyValue(
        key_columns
            .iter()
            .map(|name| {
                let value = index
                    .get(name)
                    .and_then(|idx| row.get(*idx))
                    .cloned()
                    .unwrap_or_default();
                (name.clone(), value)
            })
            .collect(),
    )
}

fn write_csv(path: &Path, headers: &[String], rows: &[Vec<String>]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(headers)?;
    for row in rows {
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

fn parse_diff_selection(diff_text: &str) -> Result<DiffSelection, Box<dyn Error>> {
    let mut key_columns = None;
    let mut rows = Vec::new();

    for line in diff_text.lines() {
        if let Some(parsed_key_columns) = parse_key_columns_line(line) {
            key_columns = Some(parsed_key_columns);
            continue;
        }

        if let Some(selector) = parse_selector_line(line)? {
            rows.push(selector);
        }
    }

    let key_columns = key_columns
        .filter(|columns| !columns.is_empty())
        .ok_or_else(|| String::from("diff does not contain a keys line"))?;

    let selection = DiffSelection { key_columns, rows };
    validate_row_selectors(&selection)?;
    Ok(selection)
}

fn validate_row_selectors(selection: &DiffSelection) -> Result<(), Box<dyn Error>> {
    for selector in &selection.rows {
        if selector
            .key
            .0
            .iter()
            .map(|(column, _)| column)
            .ne(selection.key_columns.iter())
        {
            return Err(String::from("diff row key does not match the keys line").into());
        }
    }
    Ok(())
}

fn parse_key_columns_line(line: &str) -> Option<Vec<String>> {
    line.strip_prefix("keys: ")
        .map(|value| value.split(", ").map(ToOwned::to_owned).collect::<Vec<_>>())
}

fn parse_selector_line(line: &str) -> Result<Option<RowSelector>, Box<dyn Error>> {
    if !line.starts_with("@@ key [") {
        return Ok(None);
    }

    let rest = &line["@@ key [".len()..];
    let Some((key_part, occurrence_part)) = rest.split_once("] occurrence ") else {
        return Err(format!("invalid diff row header: {line}").into());
    };
    let Some((occurrence_text, _tail)) = occurrence_part.split_once(' ') else {
        return Err(format!("invalid occurrence section: {line}").into());
    };

    let occurrence = occurrence_text
        .parse::<usize>()?
        .checked_sub(1)
        .ok_or_else(|| format!("occurrence must be 1 or greater: {line}"))?;

    Ok(Some(RowSelector {
        key: KeyValue(parse_key_entries(key_part)?),
        occurrence,
    }))
}

fn parse_key_entries(input: &str) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut entries = Vec::new();
    let mut pos = 0;
    let bytes = input.as_bytes();

    while pos < bytes.len() {
        let start = pos;
        while pos < bytes.len() && bytes[pos] != b'=' {
            pos += 1;
        }
        if pos >= bytes.len() {
            return Err(format!("invalid key entry: {input}").into());
        }
        let name = input[start..pos].trim().to_string();
        pos += 1;

        let (value, next_pos) = parse_quoted_value(input, pos)?;
        entries.push((name, value));
        pos = next_pos;

        if pos < bytes.len() {
            if input[pos..].starts_with(", ") {
                pos += 2;
            } else {
                return Err(format!("invalid key separator: {input}").into());
            }
        }
    }

    Ok(entries)
}

fn parse_quoted_value(input: &str, start: usize) -> Result<(String, usize), Box<dyn Error>> {
    let bytes = input.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return Err(format!("expected quoted value: {input}").into());
    }

    let mut result = String::new();
    let mut pos = start + 1;

    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' => {
                pos += 1;
                let Some(&escaped) = bytes.get(pos) else {
                    return Err(format!("unterminated escape sequence: {input}").into());
                };
                result.push(match escaped {
                    b'\\' => '\\',
                    b'"' => '"',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    other => other as char,
                });
                pos += 1;
            }
            b'"' => return Ok((result, pos + 1)),
            byte => {
                result.push(byte as char);
                pos += 1;
            }
        }
    }

    Err(format!("unterminated quoted value: {input}").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::csv_key_diff::{diff_csv, render_diff};
    use crate::tools::test_support::{temp_path, write_file, write_zst_file};
    use std::fs;
    use std::path::PathBuf;

    fn csv(path: &str, headers: &[&str], rows: &[&[&str]]) -> CsvData {
        CsvData {
            path: PathBuf::from(path),
            headers: headers.iter().map(|s| (*s).to_string()).collect(),
            rows: rows
                .iter()
                .map(|row| row.iter().map(|s| (*s).to_string()).collect())
                .collect(),
        }
    }

    #[test]
    fn parses_composite_key_selector() {
        let selector =
            parse_selector_line(r#"@@ key [id="1", sub_id="b"] occurrence 2 modified @@"#)
                .unwrap()
                .unwrap();

        assert_eq!(
            selector,
            RowSelector {
                key: KeyValue(vec![
                    (String::from("id"), String::from("1")),
                    (String::from("sub_id"), String::from("b")),
                ]),
                occurrence: 1,
            }
        );
    }

    #[test]
    fn extracts_differing_rows_from_both_sides() {
        let left = csv(
            "left.csv",
            &["id", "sub_id", "value"],
            &[&["1", "a", "10"], &["1", "b", "20"], &["2", "a", "30"]],
        );
        let right = csv(
            "right.csv",
            &["id", "sub_id", "value"],
            &[&["1", "a", "10"], &["1", "b", "25"], &["3", "a", "40"]],
        );

        let report = diff_csv(&left, &right, &[String::from("id"), String::from("sub_id")]);
        let selection = parse_diff_selection(&render_diff(&report)).unwrap();

        let left_rows = extract_rows(&left, &selection);
        let right_rows = extract_rows(&right, &selection);

        assert_eq!(
            left_rows,
            vec![
                vec![String::from("1"), String::from("b"), String::from("20")],
                vec![String::from("2"), String::from("a"), String::from("30")],
            ]
        );
        assert_eq!(
            right_rows,
            vec![
                vec![String::from("1"), String::from("b"), String::from("25")],
                vec![String::from("3"), String::from("a"), String::from("40")],
            ]
        );
    }

    #[test]
    fn parses_key_columns_line() {
        assert_eq!(
            parse_key_columns_line("keys: id, sub_id"),
            Some(vec![String::from("id"), String::from("sub_id")])
        );
    }

    #[test]
    fn rejects_diff_without_key_columns() {
        let error = parse_diff_selection("@@ header differences @@\n").unwrap_err();

        assert!(error.to_string().contains("does not contain a keys line"));
    }

    #[test]
    fn rejects_selector_with_different_key_columns() {
        let error =
            parse_diff_selection("keys: id\n@@ key [other=\"1\"] occurrence 1 modified @@\n")
                .unwrap_err();

        assert!(error.to_string().contains("does not match the keys line"));
    }

    #[test]
    fn extracts_rows_from_zst_csv_inputs() {
        let left_path = temp_path("left.csv.zst");
        let right_path = temp_path("right.csv.zst");
        let diff_path = temp_path("diff.txt");
        let output_left_path = temp_path("left_out.csv");
        let output_right_path = temp_path("right_out.csv");

        write_zst_file(&left_path, "id,value\n1,10\n2,20\n").unwrap();
        write_zst_file(&right_path, "id,value\n1,15\n2,20\n").unwrap();
        write_file(
            &diff_path,
            "--- left\n+++ right\nkeys: id\n@@ key [id=\"1\"] occurrence 1 modified @@\n- value=\"10\"\n+ value=\"15\"\n",
        )
        .unwrap();

        extract_diff_rows_to_files(
            &left_path,
            &right_path,
            &diff_path,
            &output_left_path,
            &output_right_path,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(&output_left_path).unwrap(),
            "id,value\n1,10\n"
        );
        assert_eq!(
            fs::read_to_string(&output_right_path).unwrap(),
            "id,value\n1,15\n"
        );

        let _ = fs::remove_file(left_path);
        let _ = fs::remove_file(right_path);
        let _ = fs::remove_file(diff_path);
        let _ = fs::remove_file(output_left_path);
        let _ = fs::remove_file(output_right_path);
    }
}
