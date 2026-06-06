use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvData {
    pub path: PathBuf,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderDiff {
    pub only_left: Vec<String>,
    pub only_right: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyValue(pub Vec<(String, String)>);

impl fmt::Display for KeyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = self
            .0
            .iter()
            .map(|(name, value)| format!("{name}={value:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "{rendered}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnChange {
    pub column: String,
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowChangeKind {
    OnlyLeft,
    OnlyRight,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowChange {
    pub key: KeyValue,
    pub occurrence: usize,
    pub kind: RowChangeKind,
    pub columns: Vec<ColumnChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffReport {
    pub left_path: PathBuf,
    pub right_path: PathBuf,
    pub key_columns: Vec<String>,
    pub missing_key_columns_left: Vec<String>,
    pub missing_key_columns_right: Vec<String>,
    pub header_diff: HeaderDiff,
    pub row_changes: Vec<RowChange>,
}

impl DiffReport {
    pub fn has_differences(&self) -> bool {
        !self.missing_key_columns_left.is_empty()
            || !self.missing_key_columns_right.is_empty()
            || !self.header_diff.only_left.is_empty()
            || !self.header_diff.only_right.is_empty()
            || !self.row_changes.is_empty()
    }
}

pub fn read_csv(path: &Path) -> Result<CsvData, Box<dyn Error>> {
    let mut reader = csv::Reader::from_reader(Cursor::new(read_csv_bytes(path)?));
    let headers = reader
        .headers()?
        .iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        rows.push(record.iter().map(ToOwned::to_owned).collect());
    }

    Ok(CsvData {
        path: path.to_path_buf(),
        headers,
        rows,
    })
}

fn read_csv_bytes(path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    let file = File::open(path)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    if file_name.ends_with(".tar.zst") || file_name.ends_with(".tzst") {
        return read_csv_from_tar(path, zstd::Decoder::new(file)?);
    }
    if file_name.ends_with(".tar") {
        return read_csv_from_tar(path, file);
    }
    if file_name.ends_with(".zst") {
        let mut decoder = zstd::Decoder::new(file)?;
        let mut bytes = Vec::new();
        decoder.read_to_end(&mut bytes)?;
        return Ok(bytes);
    }

    let mut bytes = Vec::new();
    let mut reader = file;
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_csv_from_tar(path: &Path, reader: impl Read) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut archive = tar::Archive::new(reader);
    let mut csv_entry = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }

        let entry_path = entry.path()?.into_owned();
        if !entry_path
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".csv")
        {
            continue;
        }

        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if csv_entry.replace((entry_path, bytes)).is_some() {
            return Err(format!(
                "archive contains multiple csv files, expected exactly one: {}",
                path.display()
            )
            .into());
        }
    }

    csv_entry
        .map(|(_, bytes)| bytes)
        .ok_or_else(|| format!("archive does not contain a csv file: {}", path.display()).into())
}

pub fn diff_csv(left: &CsvData, right: &CsvData, key_columns: &[String]) -> DiffReport {
    let left_header_set = left.headers.iter().cloned().collect::<HashSet<_>>();
    let right_header_set = right.headers.iter().cloned().collect::<HashSet<_>>();

    let missing_key_columns_left = key_columns
        .iter()
        .filter(|name| !left_header_set.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    let missing_key_columns_right = key_columns
        .iter()
        .filter(|name| !right_header_set.contains(*name))
        .cloned()
        .collect::<Vec<_>>();

    let header_diff = HeaderDiff {
        only_left: left
            .headers
            .iter()
            .filter(|name| !right_header_set.contains(*name))
            .cloned()
            .collect(),
        only_right: right
            .headers
            .iter()
            .filter(|name| !left_header_set.contains(*name))
            .cloned()
            .collect(),
    };

    let row_changes = if missing_key_columns_left.is_empty() && missing_key_columns_right.is_empty()
    {
        diff_rows(left, right, key_columns)
    } else {
        Vec::new()
    };

    DiffReport {
        left_path: left.path.clone(),
        right_path: right.path.clone(),
        key_columns: key_columns.to_vec(),
        missing_key_columns_left,
        missing_key_columns_right,
        header_diff,
        row_changes,
    }
}

fn diff_rows(left: &CsvData, right: &CsvData, key_columns: &[String]) -> Vec<RowChange> {
    let left_index = header_index(&left.headers);
    let right_index = header_index(&right.headers);
    let all_columns = merged_columns(&left.headers, &right.headers);

    let mut left_groups = group_rows(left, &left_index, key_columns);
    let mut right_groups = group_rows(right, &right_index, key_columns);
    let key_order = merged_key_order(left, right, &left_index, &right_index, key_columns);

    let mut row_changes = Vec::new();
    for key in key_order {
        let left_rows = left_groups.remove(&key).unwrap_or_default();
        let right_rows = right_groups.remove(&key).unwrap_or_default();
        let max_len = left_rows.len().max(right_rows.len());

        for occurrence in 0..max_len {
            match (left_rows.get(occurrence), right_rows.get(occurrence)) {
                (Some(left_row), Some(right_row)) => {
                    let columns = compare_columns(
                        left_row,
                        right_row,
                        &left_index,
                        &right_index,
                        &all_columns,
                    );
                    if !columns.is_empty() {
                        row_changes.push(RowChange {
                            key: key.clone(),
                            occurrence,
                            kind: RowChangeKind::Modified,
                            columns,
                        });
                    }
                }
                (Some(left_row), None) => row_changes.push(RowChange {
                    key: key.clone(),
                    occurrence,
                    kind: RowChangeKind::OnlyLeft,
                    columns: row_snapshot(left_row, &left_index, &all_columns, true),
                }),
                (None, Some(right_row)) => row_changes.push(RowChange {
                    key: key.clone(),
                    occurrence,
                    kind: RowChangeKind::OnlyRight,
                    columns: row_snapshot(right_row, &right_index, &all_columns, false),
                }),
                (None, None) => {}
            }
        }
    }

    row_changes
}

fn header_index(headers: &[String]) -> HashMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(idx, name)| (name.clone(), idx))
        .collect()
}

fn merged_columns(left: &[String], right: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut columns = Vec::new();

    for name in left.iter().chain(right.iter()) {
        if seen.insert(name.clone()) {
            columns.push(name.clone());
        }
    }

    columns
}

fn group_rows<'a>(
    csv: &'a CsvData,
    index: &HashMap<String, usize>,
    key_columns: &[String],
) -> HashMap<KeyValue, Vec<&'a Vec<String>>> {
    let mut groups = HashMap::<KeyValue, Vec<&Vec<String>>>::new();
    for row in &csv.rows {
        groups
            .entry(extract_key(row, index, key_columns))
            .or_default()
            .push(row);
    }
    groups
}

fn merged_key_order(
    left: &CsvData,
    right: &CsvData,
    left_index: &HashMap<String, usize>,
    right_index: &HashMap<String, usize>,
    key_columns: &[String],
) -> Vec<KeyValue> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();

    for row in &left.rows {
        let key = extract_key(row, left_index, key_columns);
        if seen.insert(key.clone()) {
            ordered.push(key);
        }
    }

    for row in &right.rows {
        let key = extract_key(row, right_index, key_columns);
        if seen.insert(key.clone()) {
            ordered.push(key);
        }
    }

    ordered
}

fn extract_key(row: &[String], index: &HashMap<String, usize>, key_columns: &[String]) -> KeyValue {
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

fn compare_columns(
    left_row: &[String],
    right_row: &[String],
    left_index: &HashMap<String, usize>,
    right_index: &HashMap<String, usize>,
    all_columns: &[String],
) -> Vec<ColumnChange> {
    all_columns
        .iter()
        .filter_map(|column| {
            let left = left_index
                .get(column)
                .and_then(|idx| left_row.get(*idx))
                .cloned();
            let right = right_index
                .get(column)
                .and_then(|idx| right_row.get(*idx))
                .cloned();
            (left != right).then(|| ColumnChange {
                column: column.clone(),
                left,
                right,
            })
        })
        .collect()
}

fn row_snapshot(
    row: &[String],
    index: &HashMap<String, usize>,
    all_columns: &[String],
    is_left: bool,
) -> Vec<ColumnChange> {
    all_columns
        .iter()
        .filter_map(|column| {
            let value = index.get(column).and_then(|idx| row.get(*idx)).cloned();
            value.map(|v| ColumnChange {
                column: column.clone(),
                left: is_left.then_some(v.clone()),
                right: (!is_left).then_some(v),
            })
        })
        .collect()
}

pub fn render_diff(report: &DiffReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!("--- {}", report.left_path.display()));
    lines.push(format!("+++ {}", report.right_path.display()));

    if !report.key_columns.is_empty() {
        lines.push(format!("keys: {}", report.key_columns.join(", ")));
    }

    if !report.missing_key_columns_left.is_empty() || !report.missing_key_columns_right.is_empty() {
        lines.push(String::from("@@ missing key columns @@"));
        for name in &report.missing_key_columns_left {
            lines.push(format!("- left is missing key column {name:?}"));
        }
        for name in &report.missing_key_columns_right {
            lines.push(format!("+ right is missing key column {name:?}"));
        }
    }

    if !report.header_diff.only_left.is_empty() || !report.header_diff.only_right.is_empty() {
        lines.push(String::from("@@ header differences @@"));
        for name in &report.header_diff.only_left {
            lines.push(format!("- column only in left: {name:?}"));
        }
        for name in &report.header_diff.only_right {
            lines.push(format!("+ column only in right: {name:?}"));
        }
    }

    for change in &report.row_changes {
        let title = match change.kind {
            RowChangeKind::OnlyLeft => "only in left",
            RowChangeKind::OnlyRight => "only in right",
            RowChangeKind::Modified => "modified",
        };
        lines.push(format!(
            "@@ key [{}] occurrence {} {title} @@",
            change.key,
            change.occurrence + 1
        ));

        for column in &change.columns {
            match change.kind {
                RowChangeKind::OnlyLeft => {
                    if let Some(left) = &column.left {
                        lines.push(format!("- {}={left:?}", column.column));
                    }
                }
                RowChangeKind::OnlyRight => {
                    if let Some(right) = &column.right {
                        lines.push(format!("+ {}={right:?}", column.column));
                    }
                }
                RowChangeKind::Modified => {
                    lines.push(format!(
                        "- {}={}",
                        column.column,
                        render_cell_value(column.left.as_deref())
                    ));
                    lines.push(format!(
                        "+ {}={}",
                        column.column,
                        render_cell_value(column.right.as_deref())
                    ));
                }
            }
        }
    }

    if !report.has_differences() {
        lines.push(String::from("no differences"));
    }

    lines.join("\n")
}

fn render_cell_value(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("{value:?}"),
        None => String::from("<missing>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::{temp_path, write_tar_zst_file, write_zst_file};
    use std::fs;

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
    fn reports_modified_rows_by_key() {
        let left = csv("left.csv", &["id", "name", "value"], &[&["1", "foo", "10"]]);
        let right = csv(
            "right.csv",
            &["id", "name", "value"],
            &[&["1", "foo", "20"]],
        );

        let report = diff_csv(&left, &right, &[String::from("id")]);

        assert!(report.has_differences());
        assert_eq!(report.row_changes.len(), 1);
        assert_eq!(report.row_changes[0].kind, RowChangeKind::Modified);
        assert_eq!(report.row_changes[0].columns.len(), 1);
        assert_eq!(report.row_changes[0].columns[0].column, "value");
    }

    #[test]
    fn reports_missing_header_as_difference() {
        let left = csv("left.csv", &["id", "name"], &[&["1", "foo"]]);
        let right = csv("right.csv", &["id", "value"], &[&["1", "10"]]);

        let report = diff_csv(&left, &right, &[String::from("id")]);

        assert_eq!(report.header_diff.only_left, vec![String::from("name")]);
        assert_eq!(report.header_diff.only_right, vec![String::from("value")]);
        assert_eq!(report.row_changes.len(), 1);
    }

    #[test]
    fn reports_missing_key_column() {
        let left = csv("left.csv", &["id", "name"], &[&["1", "foo"]]);
        let right = csv("right.csv", &["name"], &[&["foo"]]);

        let report = diff_csv(&left, &right, &[String::from("id")]);

        assert_eq!(report.missing_key_columns_right, vec![String::from("id")]);
        assert!(report.row_changes.is_empty());
    }

    #[test]
    fn matches_rows_by_composite_key() {
        let left = csv(
            "left.csv",
            &["id", "sub_id", "value"],
            &[&["1", "a", "10"], &["1", "b", "20"]],
        );
        let right = csv(
            "right.csv",
            &["id", "sub_id", "value"],
            &[&["1", "b", "25"], &["1", "a", "10"]],
        );

        let report = diff_csv(&left, &right, &[String::from("id"), String::from("sub_id")]);

        assert_eq!(report.row_changes.len(), 1);
        assert_eq!(report.row_changes[0].kind, RowChangeKind::Modified);
        assert_eq!(
            report.row_changes[0].key,
            KeyValue(vec![
                (String::from("id"), String::from("1")),
                (String::from("sub_id"), String::from("b")),
            ])
        );
        assert_eq!(report.row_changes[0].columns[0].column, "value");
        assert_eq!(
            report.row_changes[0].columns[0].left,
            Some(String::from("20"))
        );
        assert_eq!(
            report.row_changes[0].columns[0].right,
            Some(String::from("25"))
        );
    }

    #[test]
    fn reads_zst_csv() {
        let path = temp_path("sample.csv.zst");
        write_zst_file(&path, "id,value\n1,10\n").unwrap();

        let csv = read_csv(&path).unwrap();

        assert_eq!(csv.headers, vec![String::from("id"), String::from("value")]);
        assert_eq!(csv.rows, vec![vec![String::from("1"), String::from("10")]]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reads_tar_zst_csv() {
        let path = temp_path("sample.tar.zst");
        write_tar_zst_file(&path, "nested/data.csv", "id,value\n1,10\n").unwrap();

        let csv = read_csv(&path).unwrap();

        assert_eq!(csv.headers, vec![String::from("id"), String::from("value")]);
        assert_eq!(csv.rows, vec![vec![String::from("1"), String::from("10")]]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_tar_zst_with_multiple_csv_files() {
        let path = temp_path("multiple.tar.zst");
        {
            use std::fs::File;

            let file = File::create(&path).unwrap();
            let encoder = zstd::Encoder::new(file, 0).unwrap();
            let mut builder = tar::Builder::new(encoder);

            for (name, contents) in [("a.csv", "id\n1\n"), ("b.csv", "id\n2\n")] {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                header.set_size(contents.len() as u64);
                header.set_cksum();
                builder
                    .append_data(&mut header, name, contents.as_bytes())
                    .unwrap();
            }

            let encoder = builder.into_inner().unwrap();
            encoder.finish().unwrap();
        }

        let error = read_csv(&path).unwrap_err();

        assert!(error.to_string().contains("multiple csv files"));

        let _ = fs::remove_file(path);
    }
}
