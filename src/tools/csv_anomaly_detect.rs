use std::error::Error;
use std::path::Path;

use super::csv_key_diff::read_csv;

const LOCAL_SCORE_THRESHOLD: f64 = 2.0;
const NUMERICAL_NOISE_FLOOR: f64 = 1e-9;

const ANOMALY_POSITIVE_SPIKE: u32 = 1;
const ANOMALY_NEGATIVE_SPIKE: u32 = 2;

pub fn run(input_path: &Path, x_column: &str, y_columns: &[String]) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let labeled_rows = detect_anomalies(&csv.headers, &csv.rows, x_column, y_columns)?;
    print!(
        "{}",
        render_csv(&csv.headers, &csv.rows, y_columns, &labeled_rows)?
    );
    Ok(())
}

fn detect_anomalies(
    headers: &[String],
    rows: &[Vec<String>],
    x_column: &str,
    y_columns: &[String],
) -> Result<Vec<Vec<u32>>, Box<dyn Error>> {
    validate_output_headers(headers, y_columns)?;
    let x_index = find_column_index(headers, x_column)?;
    let x_values = rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| parse_cell(row, x_index, x_column, row_index))
        .collect::<Result<Vec<_>, _>>()?;
    validate_strictly_increasing(&x_values, x_column)?;

    let mut masks = vec![vec![0; y_columns.len()]; rows.len()];

    for (column_offset, y_column) in y_columns.iter().enumerate() {
        let y_index = find_column_index(headers, y_column)?;
        let y_values = rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| parse_cell(row, y_index, y_column, row_index))
            .collect::<Result<Vec<_>, _>>()?;
        let residuals = compute_residuals(&x_values, &y_values);
        let scores = local_scores(&residuals);

        for row_index in 0..rows.len() {
            let Some(residual) = residuals[row_index] else {
                continue;
            };
            let Some(score) = scores[row_index] else {
                continue;
            };
            if score < LOCAL_SCORE_THRESHOLD || !is_local_peak(&residuals, row_index) {
                continue;
            }

            masks[row_index][column_offset] = anomaly_mask(residual);
        }
    }

    Ok(masks)
}

fn validate_output_headers(headers: &[String], y_columns: &[String]) -> Result<(), Box<dyn Error>> {
    for label in anomaly_label_headers(y_columns) {
        if headers.iter().any(|header| header == &label) {
            return Err(format!("output header already exists: {label}").into());
        }
    }

    Ok(())
}

fn anomaly_label_headers(y_columns: &[String]) -> Vec<String> {
    y_columns
        .iter()
        .map(|column| format!("{column}_anomaly_mask"))
        .collect()
}

fn anomaly_mask(residual: f64) -> u32 {
    if residual > 0.0 {
        ANOMALY_POSITIVE_SPIKE
    } else if residual < 0.0 {
        ANOMALY_NEGATIVE_SPIKE
    } else {
        0
    }
}

fn validate_strictly_increasing(x_values: &[f64], x_column: &str) -> Result<(), Box<dyn Error>> {
    for index in 1..x_values.len() {
        if x_values[index - 1] >= x_values[index] {
            return Err(format!(
                "{x_column} must be strictly increasing for anomaly detection: row {} has {} after {}",
                index + 1,
                x_values[index],
                x_values[index - 1]
            )
            .into());
        }
    }

    Ok(())
}

fn is_local_peak(residuals: &[Option<f64>], index: usize) -> bool {
    let Some(current) = residuals[index].map(f64::abs) else {
        return false;
    };

    let start = index.saturating_sub(1);
    let end = (index + 1).min(residuals.len().saturating_sub(1));

    (start..=end)
        .filter(|neighbor_index| *neighbor_index != index)
        .filter_map(|neighbor_index| residuals[neighbor_index].map(f64::abs))
        .all(|neighbor| current > neighbor)
}

fn compute_residuals(x_values: &[f64], y_values: &[f64]) -> Vec<Option<f64>> {
    let mut residuals = vec![None; x_values.len()];

    for index in 1..x_values.len().saturating_sub(1) {
        let previous_x = x_values[index - 1];
        let current_x = x_values[index];
        let next_x = x_values[index + 1];
        let previous_y = y_values[index - 1];
        let current_y = y_values[index];
        let next_y = y_values[index + 1];

        let dx = next_x - previous_x;
        if !dx.is_finite() || dx == 0.0 {
            continue;
        }

        let expected = previous_y + (current_x - previous_x) / dx * (next_y - previous_y);
        residuals[index] = Some(current_y - expected);
    }

    residuals
}

fn local_scores(residuals: &[Option<f64>]) -> Vec<Option<f64>> {
    let mut scores = vec![None; residuals.len()];

    for index in 0..residuals.len() {
        let Some(residual) = residuals[index] else {
            continue;
        };

        let start = index.saturating_sub(2);
        let end = (index + 2).min(residuals.len().saturating_sub(1));
        let neighbors = (start..=end)
            .filter(|neighbor_index| *neighbor_index != index)
            .filter_map(|neighbor_index| residuals[neighbor_index].map(f64::abs))
            .collect::<Vec<_>>();

        if neighbors.is_empty() {
            continue;
        }

        let baseline = median(neighbors);
        let local_scale = baseline.max(residual.abs()).max(1.0);
        let noise_floor = local_scale * NUMERICAL_NOISE_FLOOR;
        scores[index] = Some(if residual.abs() <= noise_floor {
            0.0
        } else if baseline == 0.0 {
            f64::INFINITY
        } else {
            residual.abs() / baseline
        });
    }

    scores
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn render_csv(
    headers: &[String],
    rows: &[Vec<String>],
    y_columns: &[String],
    masks: &[Vec<u32>],
) -> Result<String, Box<dyn Error>> {
    let mut writer = csv::Writer::from_writer(Vec::new());

    let mut output_headers = headers.to_vec();
    output_headers.extend(anomaly_label_headers(y_columns));
    writer.write_record(&output_headers)?;

    for (row, row_masks) in rows.iter().zip(masks.iter()) {
        let mut output_row = row.clone();
        output_row.extend(row_masks.iter().map(u32::to_string));
        writer.write_record(&output_row)?;
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

fn parse_cell(
    row: &[String],
    index: usize,
    column_name: &str,
    row_index: usize,
) -> Result<f64, Box<dyn Error>> {
    let value = row
        .get(index)
        .ok_or_else(|| format!("row is missing column value: {column_name}"))?;
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
    use crate::tools::test_support::{temp_path, write_tar_zst_file};
    use std::fs;

    #[test]
    fn computes_local_trend_residuals() {
        let x = vec![0.0, 1.0, 2.0, 3.0, 4.0];
        let y = vec![0.0, 1.0, 20.0, 3.0, 4.0];
        let residuals = compute_residuals(&x, &y);

        assert_eq!(residuals[0], None);
        assert_eq!(residuals[2], Some(18.0));
        assert_eq!(residuals[4], None);
    }

    #[test]
    fn labels_positive_and_negative_spikes() {
        let headers = vec![
            String::from("t"),
            String::from("y_up"),
            String::from("y_down"),
        ];
        let rows = vec![
            vec![String::from("0"), String::from("0"), String::from("0")],
            vec![String::from("1"), String::from("1"), String::from("1")],
            vec![String::from("2"), String::from("20"), String::from("-20")],
            vec![String::from("3"), String::from("3"), String::from("3")],
            vec![String::from("4"), String::from("4"), String::from("4")],
            vec![String::from("5"), String::from("5"), String::from("5")],
            vec![String::from("6"), String::from("6"), String::from("6")],
        ];

        let masks = detect_anomalies(
            &headers,
            &rows,
            "t",
            &[String::from("y_up"), String::from("y_down")],
        )
        .unwrap();

        assert_eq!(masks[2][0], ANOMALY_POSITIVE_SPIKE);
        assert_eq!(masks[2][1], ANOMALY_NEGATIVE_SPIKE);
        assert_eq!(masks[1][0], 0);
        assert_eq!(masks[3][1], 0);
    }

    #[test]
    fn rejects_unsorted_x_values() {
        let headers = vec![String::from("t"), String::from("y")];
        let rows = vec![
            vec![String::from("0"), String::from("0")],
            vec![String::from("2"), String::from("2")],
            vec![String::from("1"), String::from("1")],
        ];

        let error = detect_anomalies(&headers, &rows, "t", &[String::from("y")]).unwrap_err();

        assert!(error.to_string().contains("strictly increasing"));
    }

    #[test]
    fn rejects_conflicting_output_header() {
        let headers = vec![
            String::from("t"),
            String::from("y"),
            String::from("y_anomaly_mask"),
        ];
        let rows = vec![
            vec![String::from("0"), String::from("0"), String::from("0")],
            vec![String::from("1"), String::from("1"), String::from("0")],
            vec![String::from("2"), String::from("2"), String::from("0")],
        ];

        let error = detect_anomalies(&headers, &rows, "t", &[String::from("y")]).unwrap_err();

        assert!(error.to_string().contains("output header already exists"));
    }

    #[test]
    fn ignores_numerical_noise_on_flat_residual_baseline() {
        let headers = vec![String::from("t"), String::from("y")];
        let rows = vec![
            vec![String::from("0"), String::from("0.0")],
            vec![String::from("1"), String::from("1.0")],
            vec![String::from("2"), String::from("2.000000000001")],
            vec![String::from("3"), String::from("3.0")],
            vec![String::from("4"), String::from("4.0")],
        ];

        let masks = detect_anomalies(&headers, &rows, "t", &[String::from("y")]).unwrap();

        assert!(masks.iter().all(|row| row == &[0]));
    }

    #[test]
    fn renders_csv_output_with_anomaly_columns() {
        let output = render_csv(
            &[String::from("time"), String::from("signal")],
            &[vec![String::from("1.0"), String::from("10.0")]],
            &[String::from("signal")],
            &[vec![ANOMALY_POSITIVE_SPIKE]],
        )
        .unwrap();

        assert_eq!(output, "time,signal,signal_anomaly_mask\n1.0,10.0,1\n");
    }

    #[test]
    fn reads_tar_zst_csv_input() {
        let path = temp_path("anomaly.tar.zst");
        write_tar_zst_file(&path, "input.csv", "t,y\n0,0\n1,1\n2,20\n3,3\n4,4\n").unwrap();

        let csv = read_csv(&path).unwrap();
        let masks = detect_anomalies(&csv.headers, &csv.rows, "t", &[String::from("y")]).unwrap();

        assert_eq!(masks[2][0], ANOMALY_POSITIVE_SPIKE);

        let _ = fs::remove_file(path);
    }
}
