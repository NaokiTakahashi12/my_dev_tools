use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use super::csv_key_diff::{CsvData, read_csv};

pub fn run(
    input_path: &Path,
    label_column: &str,
    output_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    split_csv_by_label(
        &csv,
        label_column,
        output_dir.unwrap_or_else(|| Path::new(".")),
    )
}

fn split_csv_by_label(
    csv: &CsvData,
    label_column: &str,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let label_index = find_column_index(&csv.headers, label_column)?;
    let output_headers = csv
        .headers
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != label_index)
        .map(|(_, header)| header.clone())
        .collect::<Vec<_>>();

    let mut rows_by_label = BTreeMap::<String, Vec<Vec<String>>>::new();
    for (row_index, row) in csv.rows.iter().enumerate() {
        let label = row
            .get(label_index)
            .ok_or_else(|| format!("row is missing column value: {label_column}"))?
            .trim();
        if label.is_empty() {
            return Err(format!(
                "row {} column {label_column} must not be empty",
                row_index + 1
            )
            .into());
        }

        let filtered_row = row
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != label_index)
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        rows_by_label
            .entry(label.to_string())
            .or_default()
            .push(filtered_row);
    }

    fs::create_dir_all(output_dir)?;
    let base_name = base_name_for_output(&csv.path);
    let mut outputs = Vec::with_capacity(rows_by_label.len());
    for (index, (label, rows)) in rows_by_label.into_iter().enumerate() {
        let output_path = output_dir.join(format!(
            "{base_name}_{:03}_{}.csv",
            index + 1,
            sanitize_label_for_path(&label)
        ));
        write_csv(&output_path, &output_headers, &rows)?;
        outputs.push(output_path);
    }

    Ok(outputs)
}

fn find_column_index(headers: &[String], name: &str) -> Result<usize, Box<dyn Error>> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format!("column not found: {name}").into())
}

fn base_name_for_output(path: &Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let lower = file_name.to_ascii_lowercase();
    for suffix in [".tar.zst", ".tzst", ".tar", ".zst", ".csv"] {
        if lower.ends_with(suffix) {
            return file_name[..file_name.len() - suffix.len()].to_string();
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output")
        .to_string()
}

fn sanitize_label_for_path(label: &str) -> String {
    let sanitized = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if sanitized.is_empty() {
        String::from("label")
    } else {
        sanitized
    }
}

fn write_csv(path: &Path, headers: &[String], rows: &[Vec<String>]) -> Result<(), Box<dyn Error>> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(headers)?;
    for row in rows {
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::{temp_path, write_file, write_zst_file};

    #[test]
    fn splits_csv_by_label_and_drops_label_column() {
        let csv = CsvData {
            path: Path::new("input.csv").to_path_buf(),
            headers: vec![
                String::from("label"),
                String::from("stamp"),
                String::from("signal"),
            ],
            rows: vec![
                vec![String::from("beta"), String::from("2"), String::from("5")],
                vec![String::from("alpha"), String::from("1"), String::from("3")],
                vec![String::from("alpha"), String::from("4"), String::from("8")],
            ],
        };
        let output_dir = temp_path("label_split");

        let outputs = split_csv_by_label(&csv, "label", &output_dir).unwrap();

        assert_eq!(outputs.len(), 2);
        let alpha = fs::read_to_string(&outputs[0]).unwrap();
        let beta = fs::read_to_string(&outputs[1]).unwrap();
        assert!(alpha.starts_with("stamp,signal\n"));
        assert!(alpha.contains("1,3\n4,8\n"));
        assert!(beta.contains("2,5\n"));
        fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn uses_input_stem_and_sanitized_label_in_output_name() {
        let csv = CsvData {
            path: Path::new("archive.tar.zst").to_path_buf(),
            headers: vec![String::from("label"), String::from("value")],
            rows: vec![vec![String::from("a/b"), String::from("1")]],
        };
        let output_dir = temp_path("label_name");

        let outputs = split_csv_by_label(&csv, "label", &output_dir).unwrap();

        assert_eq!(
            outputs[0].file_name().and_then(|value| value.to_str()),
            Some("archive_001_a_b.csv")
        );
        fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn reads_zst_input_and_splits_labels() {
        let input = temp_path("labels.csv.zst");
        let output_dir = temp_path("labels_out");
        write_zst_file(&input, "label,t,v\nx,1,10\ny,2,20\n").unwrap();

        let outputs = run(&input, "label", Some(&output_dir)).unwrap();

        assert_eq!(outputs.len(), 2);
        let first = fs::read_to_string(&outputs[0]).unwrap();
        assert!(first.starts_with("t,v\n"));
        fs::remove_file(input).unwrap();
        fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn rejects_empty_label() {
        let input = temp_path("labels.csv");
        let output_dir = temp_path("labels_err");
        write_file(&input, "label,t\n,1\n").unwrap();

        let error = run(&input, "label", Some(&output_dir)).unwrap_err();

        assert!(error.to_string().contains("must not be empty"));
        fs::remove_file(input).unwrap();
    }
}
