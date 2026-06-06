use std::error::Error;
use std::path::Path;

use super::csv_key_diff::read_csv;

#[derive(Debug, Clone, PartialEq)]
struct OutputRow {
    time: String,
    value: String,
    pseudo_diff: String,
}

pub fn run(
    input_path: &Path,
    time_column: &str,
    value_column: &str,
    time_scale: f64,
    output_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let rows = compute_pseudo_diff_rows(
        &csv.headers,
        &csv.rows,
        time_column,
        value_column,
        time_scale,
    )?;
    let rendered = render_csv(&rows, time_column, value_column)?;

    print!("{rendered}");

    if let Some(path) = output_path {
        std::fs::write(path, &rendered)?;
    }

    Ok(())
}

fn compute_pseudo_diff_rows(
    headers: &[String],
    rows: &[Vec<String>],
    time_column: &str,
    value_column: &str,
    time_scale: f64,
) -> Result<Vec<OutputRow>, Box<dyn Error>> {
    validate_time_scale(time_scale)?;
    let time_index = find_column_index(headers, time_column)?;
    let value_index = find_column_index(headers, value_column)?;
    let mut output = Vec::new();
    let mut previous: Option<(f64, f64)> = None;

    for (row_index, row) in rows.iter().enumerate() {
        let time_text = get_cell(row, time_index, time_column)?.to_string();
        let value_text = get_cell(row, value_index, value_column)?.to_string();
        let time = parse_f64_cell(&time_text, time_column, row_index)?;
        let value = parse_f64_cell(&value_text, value_column, row_index)?;

        let pseudo_diff = if let Some((prev_time, prev_value)) = previous {
            let delta_time = (time - prev_time) * time_scale;
            if !delta_time.is_finite() || delta_time == 0.0 {
                return Err(format!(
                    "time delta became non-finite or zero at row {}: {}",
                    row_index + 1,
                    time_text
                )
                .into());
            }
            ((value - prev_value) / delta_time).to_string()
        } else {
            String::new()
        };

        output.push(OutputRow {
            time: time_text,
            value: value_text,
            pseudo_diff,
        });
        previous = Some((time, value));
    }

    Ok(output)
}

fn render_csv(
    rows: &[OutputRow],
    time_column: &str,
    value_column: &str,
) -> Result<String, Box<dyn Error>> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer.write_record([time_column, value_column, "pseudo_diff"])?;
    for row in rows {
        writer.write_record([&row.time, &row.value, &row.pseudo_diff])?;
    }
    let bytes = writer.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

fn find_column_index(headers: &[String], name: &str) -> Result<usize, Box<dyn Error>> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format!("column not found: {name}").into())
}

fn get_cell<'a>(
    row: &'a [String],
    index: usize,
    column_name: &str,
) -> Result<&'a str, Box<dyn Error>> {
    row.get(index)
        .map(|value| value.as_str())
        .ok_or_else(|| format!("row is missing column value: {column_name}").into())
}

fn validate_time_scale(time_scale: f64) -> Result<(), Box<dyn Error>> {
    if !time_scale.is_finite() || time_scale == 0.0 {
        return Err(String::from("time_scale must be finite and non-zero").into());
    }
    Ok(())
}

fn parse_f64_cell(value: &str, column_name: &str, row_index: usize) -> Result<f64, Box<dyn Error>> {
    let parsed = value.parse::<f64>().map_err(|err| {
        format!(
            "failed to parse row {} column {column_name}: {err}",
            row_index + 1
        )
    })?;
    if !parsed.is_finite() {
        return Err(format!(
            "row {} column {column_name} must be finite, got {value}",
            row_index + 1
        )
        .into());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::{temp_path, write_zst_file};
    use std::fs;

    #[test]
    fn computes_pseudo_diff_rows() {
        let headers = vec![String::from("t"), String::from("x")];
        let rows = vec![
            vec![String::from("0.0"), String::from("1.0")],
            vec![String::from("0.5"), String::from("2.0")],
            vec![String::from("1.0"), String::from("5.0")],
        ];

        let output = compute_pseudo_diff_rows(&headers, &rows, "t", "x", 2.0).unwrap();

        assert_eq!(
            output,
            vec![
                OutputRow {
                    time: String::from("0.0"),
                    value: String::from("1.0"),
                    pseudo_diff: String::new(),
                },
                OutputRow {
                    time: String::from("0.5"),
                    value: String::from("2.0"),
                    pseudo_diff: String::from("1"),
                },
                OutputRow {
                    time: String::from("1.0"),
                    value: String::from("5.0"),
                    pseudo_diff: String::from("3"),
                },
            ]
        );
    }

    #[test]
    fn renders_csv_output() {
        let rendered = render_csv(
            &[OutputRow {
                time: String::from("0.0"),
                value: String::from("1.0"),
                pseudo_diff: String::new(),
            }],
            "t",
            "x",
        )
        .unwrap();

        assert_eq!(rendered, "t,x,pseudo_diff\n0.0,1.0,\n");
    }

    #[test]
    fn rejects_non_finite_time_scale() {
        let headers = vec![String::from("t"), String::from("x")];
        let rows = vec![vec![String::from("0.0"), String::from("1.0")]];

        assert!(compute_pseudo_diff_rows(&headers, &rows, "t", "x", f64::NAN).is_err());
    }

    #[test]
    fn rejects_non_finite_cell_value() {
        let headers = vec![String::from("t"), String::from("x")];
        let rows = vec![
            vec![String::from("0.0"), String::from("1.0")],
            vec![String::from("1.0"), String::from("NaN")],
        ];

        assert!(compute_pseudo_diff_rows(&headers, &rows, "t", "x", 1.0).is_err());
    }

    #[test]
    fn reads_zst_csv_input() {
        let path = temp_path("pseudo.csv.zst");
        write_zst_file(&path, "t,x\n0.0,1.0\n1.0,3.0\n").unwrap();

        let csv = read_csv(&path).unwrap();
        let output = compute_pseudo_diff_rows(&csv.headers, &csv.rows, "t", "x", 1.0).unwrap();

        assert_eq!(output[1].pseudo_diff, String::from("2"));

        let _ = fs::remove_file(path);
    }
}
