use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;
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
    let mut reader = csv::Reader::from_path(path)?;
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
}
