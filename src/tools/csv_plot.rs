use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io::stdout;
use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use plotters::prelude::{
    BLACK, BitMapBackend, ChartBuilder, IntoDrawingArea, IntoFont, LineSeries, PathElement,
    RGBColor, Rectangle, WHITE,
};
use plotters::style::{Color as PlottersColor, FontStyle, register_font};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph};
use ratatui::{Frame, Terminal};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use super::csv_key_diff::{CsvData, read_csv};

const HORIZONTAL_MARGIN: u16 = 12;
pub const DEFAULT_IMAGE_SIZE: (u32, u32) = (1600, 900);
const MAX_STATE_DEFINITIONS: usize = 3;
const IMAGE_FONT_FAMILY: &str = "M PLUS 1";
const IMAGE_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/third_party/mplus-fonts/fonts/MPLUS1/ttf/MPLUS1-Regular.ttf"
));
const SERIES_COLORS: [Color; 6] = [
    Color::Cyan,
    Color::Yellow,
    Color::Green,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

#[derive(Debug, Clone, PartialEq)]
struct PlotData {
    kind: PlotKind,
    x_axis: AxisDescriptor,
    y_axis_label: String,
    title: String,
    series: Vec<PlotSeries>,
    derivative_series: Vec<PlotSeries>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    derivative_y_bounds: [f64; 2],
    state_bands: Option<StateBands>,
    allow_derivative_panel: bool,
    allow_fft_panel: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum PlotKind {
    Line,
    HeatmapSource(HeatmapSource),
}

#[derive(Debug, Clone, PartialEq)]
struct PlotConfig {
    kind: PlotKind,
    x_axis: AxisDescriptor,
    y_axis_label: String,
    title: String,
    allow_derivative_panel: bool,
    allow_fft_panel: bool,
}

#[derive(Debug, Clone, Copy)]
struct ChartView<'a> {
    series: &'a [PlotSeries],
    x_axis: &'a AxisDescriptor,
    y_label: &'a str,
    title: &'a str,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
}

#[derive(Debug, Clone, Copy)]
struct HeatmapView<'a> {
    heatmap_source: &'a HeatmapSource,
    x_axis: &'a AxisDescriptor,
    y_label: &'a str,
    title: &'a str,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
}

#[derive(Debug, Clone)]
struct HeatmapRenderData {
    rows: Vec<Line<'static>>,
    value_bounds: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
struct HeatmapSource {
    points: Vec<(f64, f64)>,
    sample_spacing: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct AxisDescriptor {
    label: String,
    kind: AxisKind,
}

#[derive(Debug, Clone, PartialEq)]
enum AxisKind {
    Numeric,
    Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
struct PlotSeries {
    name: String,
    points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
struct StateBands {
    definitions: Vec<StateDefinition>,
    tracks: Vec<StateTrack>,
}

#[derive(Debug, Clone, PartialEq)]
struct StateTrack {
    label: Option<String>,
    points: Vec<(f64, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
struct StateDefinition {
    bit: u64,
    label: String,
    color: StateColor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StateColor(u8, u8, u8);

#[derive(Debug, Clone, PartialEq)]
struct Viewport {
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
struct PlotState {
    main_viewport: Viewport,
    derivative_y_bounds: [f64; 2],
    fft_y_bounds: [f64; 2],
    show_derivative: bool,
    show_fft: bool,
}

pub fn run_xy(
    input_path: &Path,
    x_column: &str,
    y_columns: &[String],
) -> Result<(), Box<dyn Error>> {
    run_xy_with_state(input_path, x_column, y_columns, None, None)
}

pub fn run_xy_with_state(
    input_path: &Path,
    x_column: &str,
    y_columns: &[String],
    state_column: Option<&str>,
    state_csv_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let state_config = state_config_from_args(state_column, state_csv_path)?;
    let plot = load_xy_plot_data_with_state(&csv, x_column, y_columns, state_config)?;
    run_plot(plot)
}

pub fn run_labeled_series(
    input_path: &Path,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
) -> Result<(), Box<dyn Error>> {
    run_labeled_series_with_state(
        input_path,
        label_column,
        timestamp_column,
        value_column,
        None,
        None,
    )
}

pub fn run_labeled_series_with_state(
    input_path: &Path,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
    state_column: Option<&str>,
    state_csv_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let state_config = state_config_from_args(state_column, state_csv_path)?;
    let plot = load_labeled_series_plot_data_with_state(
        &csv,
        label_column,
        timestamp_column,
        value_column,
        state_config,
    )?;
    run_plot(plot)
}

pub fn run_power_spectrum(
    input_path: &Path,
    x_column: &str,
    y_column: &str,
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let plot = load_power_spectrum_plot_data(&csv, x_column, y_column)?;
    run_plot(plot)
}

pub fn save_image_xy(
    input_path: &Path,
    x_column: &str,
    y_columns: &[String],
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    save_image_xy_with_state(input_path, x_column, y_columns, None, None, output_path)
}

pub fn save_image_xy_with_state(
    input_path: &Path,
    x_column: &str,
    y_columns: &[String],
    state_column: Option<&str>,
    state_csv_path: Option<&Path>,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    save_image_xy_with_state_and_size(
        input_path,
        x_column,
        y_columns,
        state_column,
        state_csv_path,
        output_path,
        DEFAULT_IMAGE_SIZE,
    )
}

pub fn save_image_xy_with_state_and_size(
    input_path: &Path,
    x_column: &str,
    y_columns: &[String],
    state_column: Option<&str>,
    state_csv_path: Option<&Path>,
    output_path: &Path,
    image_size: (u32, u32),
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let state_config = state_config_from_args(state_column, state_csv_path)?;
    let plot = load_xy_plot_data_with_state(&csv, x_column, y_columns, state_config)?;
    save_line_plot_image(&plot, output_path, image_size)
}

pub fn save_image_labeled_series(
    input_path: &Path,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    save_image_labeled_series_with_state(
        input_path,
        label_column,
        timestamp_column,
        value_column,
        None,
        None,
        output_path,
    )
}

pub fn save_image_labeled_series_with_state_and_size(
    input_path: &Path,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
    state: Option<(&str, &Path)>,
    output_path: &Path,
    image_size: (u32, u32),
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let state_config = match state {
        Some((column, definitions)) => state_config_from_args(Some(column), Some(definitions))?,
        None => None,
    };
    let plot = load_labeled_series_plot_data_with_state(
        &csv,
        label_column,
        timestamp_column,
        value_column,
        state_config,
    )?;
    save_line_plot_image(&plot, output_path, image_size)
}

pub fn save_image_labeled_series_with_state(
    input_path: &Path,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
    state_column: Option<&str>,
    state_csv_path: Option<&Path>,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    save_image_labeled_series_with_state_and_size(
        input_path,
        label_column,
        timestamp_column,
        value_column,
        state_column.zip(state_csv_path),
        output_path,
        DEFAULT_IMAGE_SIZE,
    )
}

fn run_plot(plot: PlotData) -> Result<(), Box<dyn Error>> {
    let mut terminal = setup_terminal()?;
    let draw_result = draw_plot(&mut terminal, &plot);
    let restore_result = restore_terminal(&mut terminal);

    match (draw_result, restore_result) {
        (Err(draw_error), _) => Err(draw_error),
        (Ok(()), Err(restore_error)) => Err(restore_error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn state_config_from_args<'a>(
    state_column: Option<&'a str>,
    state_csv_path: Option<&'a Path>,
) -> Result<Option<(&'a str, &'a Path)>, Box<dyn Error>> {
    match (state_column, state_csv_path) {
        (Some(column), Some(path)) => Ok(Some((column, path))),
        (None, None) => Ok(None),
        _ => Err(String::from("--state-column and --state-csv must be specified together").into()),
    }
}

#[cfg(test)]
fn load_xy_plot_data(
    csv: &CsvData,
    x_column: &str,
    y_columns: &[String],
) -> Result<PlotData, Box<dyn Error>> {
    load_xy_plot_data_with_state(csv, x_column, y_columns, None)
}

fn load_xy_plot_data_with_state(
    csv: &CsvData,
    x_column: &str,
    y_columns: &[String],
    state_config: Option<(&str, &Path)>,
) -> Result<PlotData, Box<dyn Error>> {
    let x_index = find_column_index(&csv.headers, x_column)?;
    if y_columns.is_empty() {
        return Err(String::from("csv_plot requires at least one y column").into());
    }

    let y_indices = y_columns
        .iter()
        .map(|name| find_column_index(&csv.headers, name).map(|index| (name.clone(), index)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut series = y_indices
        .iter()
        .map(|(name, _)| PlotSeries {
            name: name.clone(),
            points: Vec::with_capacity(csv.rows.len()),
        })
        .collect::<Vec<_>>();

    let mut y_values = Vec::with_capacity(csv.rows.len() * y_indices.len());
    let mut x_values = Vec::with_capacity(csv.rows.len());

    for (row_index, row) in csv.rows.iter().enumerate() {
        let x = parse_f64_cell(row, x_index, x_column, row_index)?;
        x_values.push(x);
        for (series_index, (name, y_index)) in y_indices.iter().enumerate() {
            let y = parse_f64_cell(row, *y_index, name, row_index)?;
            series[series_index].points.push((x, y));
            y_values.push(y);
        }
    }

    let mut plot = build_plot_data(
        PlotConfig {
            kind: PlotKind::Line,
            x_axis: AxisDescriptor {
                label: x_column.to_string(),
                kind: AxisKind::Numeric,
            },
            y_axis_label: "y".to_string(),
            title: format!("{} vs {}", y_columns.join(", "), x_column),
            allow_derivative_panel: true,
            allow_fft_panel: true,
        },
        series,
        x_values,
        y_values,
    )?;
    plot.state_bands = load_state_bands(
        csv,
        x_index,
        x_column,
        &AxisKind::Numeric,
        None,
        state_config,
    )?;
    Ok(plot)
}

#[cfg(test)]
fn load_labeled_series_plot_data(
    csv: &CsvData,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
) -> Result<PlotData, Box<dyn Error>> {
    load_labeled_series_plot_data_with_state(
        csv,
        label_column,
        timestamp_column,
        value_column,
        None,
    )
}

fn load_labeled_series_plot_data_with_state(
    csv: &CsvData,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
    state_config: Option<(&str, &Path)>,
) -> Result<PlotData, Box<dyn Error>> {
    let label_index = find_column_index(&csv.headers, label_column)?;
    let timestamp_index = find_column_index(&csv.headers, timestamp_column)?;
    let value_index = find_column_index(&csv.headers, value_column)?;

    let mut grouped = BTreeMap::<String, Vec<(f64, f64)>>::new();
    let mut x_values = Vec::with_capacity(csv.rows.len());
    let mut y_values = Vec::with_capacity(csv.rows.len());

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

        let x = parse_timestamp_cell(row, timestamp_index, timestamp_column, row_index)?;
        let y = parse_f64_cell(row, value_index, value_column, row_index)?;
        grouped.entry(label.to_string()).or_default().push((x, y));
        x_values.push(x);
        y_values.push(y);
    }

    if grouped.is_empty() {
        return Err(String::from("csv_plot requires at least one row").into());
    }

    let mut series = grouped
        .into_iter()
        .map(|(name, mut points)| {
            points.sort_by(|left, right| left.0.total_cmp(&right.0));
            PlotSeries { name, points }
        })
        .collect::<Vec<_>>();
    series.sort_by(|left, right| left.name.cmp(&right.name));

    let mut plot = build_plot_data(
        PlotConfig {
            kind: PlotKind::Line,
            x_axis: AxisDescriptor {
                label: timestamp_column.to_string(),
                kind: AxisKind::Timestamp,
            },
            y_axis_label: value_column.to_string(),
            title: format!("{value_column} vs {timestamp_column}"),
            allow_derivative_panel: true,
            allow_fft_panel: true,
        },
        series,
        x_values,
        y_values,
    )?;
    plot.state_bands = load_state_bands(
        csv,
        timestamp_index,
        timestamp_column,
        &AxisKind::Timestamp,
        Some((label_index, label_column)),
        state_config,
    )?;
    Ok(plot)
}

fn load_power_spectrum_plot_data(
    csv: &CsvData,
    x_column: &str,
    y_column: &str,
) -> Result<PlotData, Box<dyn Error>> {
    let x_index = find_column_index(&csv.headers, x_column)?;
    let y_index = find_column_index(&csv.headers, y_column)?;
    let mut points = Vec::with_capacity(csv.rows.len());

    for (row_index, row) in csv.rows.iter().enumerate() {
        let x = parse_f64_cell(row, x_index, x_column, row_index)?;
        let y = parse_f64_cell(row, y_index, y_column, row_index)?;
        points.push((x, y));
    }

    if points.len() < 4 {
        return Err(String::from("csv_power_spectrum requires at least four rows").into());
    }

    points.sort_by(|left, right| left.0.total_cmp(&right.0));
    let sample_spacing = infer_sample_spacing(&points)?;
    let nyquist_frequency = 0.5 / sample_spacing;
    let x_bounds = axis_bounds(points.iter().map(|(x, _)| *x));
    let y_bounds = [0.0, nyquist_frequency.max(1e-9)];

    Ok(PlotData {
        kind: PlotKind::HeatmapSource(HeatmapSource {
            points,
            sample_spacing,
        }),
        x_axis: AxisDescriptor {
            label: x_column.to_string(),
            kind: AxisKind::Numeric,
        },
        y_axis_label: String::from("frequency"),
        title: format!("spectrogram({y_column})"),
        series: Vec::new(),
        derivative_series: Vec::new(),
        x_bounds,
        y_bounds,
        derivative_y_bounds: [0.0, 1.0],
        state_bands: None,
        allow_derivative_panel: false,
        allow_fft_panel: false,
    })
}

fn build_plot_data(
    config: PlotConfig,
    mut series: Vec<PlotSeries>,
    x_values: Vec<f64>,
    y_values: Vec<f64>,
) -> Result<PlotData, Box<dyn Error>> {
    if x_values.is_empty() {
        return Err(String::from("csv_plot requires at least one row").into());
    }

    for item in &mut series {
        item.points
            .sort_by(|left, right| left.0.total_cmp(&right.0));
    }

    let derivative_series = series
        .iter()
        .map(|series| PlotSeries {
            name: format!("d({})/d{}", series.name, config.x_axis.label),
            points: compute_derivative_points(&series.points),
        })
        .collect::<Vec<_>>();
    let derivative_y_values = derivative_series
        .iter()
        .flat_map(|series| series.points.iter().map(|(_, y)| *y))
        .collect::<Vec<_>>();

    Ok(PlotData {
        kind: config.kind,
        x_axis: config.x_axis,
        y_axis_label: config.y_axis_label,
        title: config.title,
        series,
        derivative_series,
        x_bounds: axis_bounds(x_values.into_iter()),
        y_bounds: axis_bounds(y_values.into_iter()),
        derivative_y_bounds: axis_bounds_or_default(derivative_y_values.into_iter(), [-1.0, 1.0]),
        state_bands: None,
        allow_derivative_panel: config.allow_derivative_panel,
        allow_fft_panel: config.allow_fft_panel,
    })
}

fn load_state_bands(
    data_csv: &CsvData,
    x_index: usize,
    x_column: &str,
    axis_kind: &AxisKind,
    label_config: Option<(usize, &str)>,
    state_config: Option<(&str, &Path)>,
) -> Result<Option<StateBands>, Box<dyn Error>> {
    let Some((state_column, definitions_path)) = state_config else {
        return Ok(None);
    };
    let state_index = find_column_index(&data_csv.headers, state_column)?;
    let definitions = load_state_definitions(definitions_path)?;
    let known_bits = definitions
        .iter()
        .fold(0_u64, |combined, definition| combined | definition.bit);
    let mut points_by_track = BTreeMap::<Option<String>, Vec<(f64, u64)>>::new();

    for (row_index, row) in data_csv.rows.iter().enumerate() {
        let x = match axis_kind {
            AxisKind::Numeric => parse_f64_cell(row, x_index, x_column, row_index)?,
            AxisKind::Timestamp => parse_timestamp_cell(row, x_index, x_column, row_index)?,
        };
        let mask = parse_state_mask(row, state_index, state_column, row_index)?;
        if mask & !known_bits != 0 {
            return Err(format!(
                "row {} column {state_column} contains bits missing from the state CSV",
                row_index + 1
            )
            .into());
        }
        let label = label_config
            .map(|(index, column)| -> Result<String, Box<dyn Error>> {
                let label = state_cell(row, index, column, row_index)?.trim();
                if label.is_empty() {
                    return Err(
                        format!("row {} column {column} must not be empty", row_index + 1).into(),
                    );
                }
                Ok(label.to_string())
            })
            .transpose()?;
        points_by_track.entry(label).or_default().push((x, mask));
    }

    Ok(Some(StateBands {
        definitions,
        tracks: points_by_track
            .into_iter()
            .map(|(label, points)| StateTrack {
                points: merge_state_points(points, label.as_deref(), axis_kind),
                label,
            })
            .collect(),
    }))
}

fn merge_state_points(
    mut points: Vec<(f64, u64)>,
    label: Option<&str>,
    axis_kind: &AxisKind,
) -> Vec<(f64, u64)> {
    points.sort_by(|left, right| left.0.total_cmp(&right.0));
    points
        .into_iter()
        .fold(Vec::new(), |mut merged, (x, mask)| {
            if let Some((previous_x, previous_mask)) = merged.last_mut()
                && *previous_x == x
            {
                eprintln!(
                    "warning: combined state masks for {} at {}",
                    label.unwrap_or("the unlabeled series"),
                    format_axis_value(x, axis_kind)
                );
                *previous_mask |= mask;
            } else {
                merged.push((x, mask));
            }
            merged
        })
}

fn load_state_definitions(path: &Path) -> Result<Vec<StateDefinition>, Box<dyn Error>> {
    let csv = read_csv(path)?;
    let bit_index = find_column_index(&csv.headers, "bit")?;
    let label_index = find_column_index(&csv.headers, "label")?;
    let color_index = csv.headers.iter().position(|header| header == "color");

    if csv.rows.is_empty() {
        return Err(String::from("state CSV requires at least one state definition").into());
    }
    if csv.rows.len() > MAX_STATE_DEFINITIONS {
        return Err(format!(
            "state CSV supports at most {MAX_STATE_DEFINITIONS} state definitions"
        )
        .into());
    }

    let mut bits = std::collections::HashSet::new();
    csv.rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let bit_text = state_cell(row, bit_index, "bit", row_index)?;
            let bit = parse_bit(bit_text, row_index)?;
            if !bits.insert(bit) {
                return Err(format!("duplicate state bit: {bit}").into());
            }

            let label = state_cell(row, label_index, "label", row_index)?.trim();
            if label.is_empty() {
                return Err(format!("row {} column label must not be empty", row_index + 1).into());
            }
            let color = color_index
                .map(|index| state_cell(row, index, "color", row_index))
                .transpose()?
                .filter(|color| !color.trim().is_empty())
                .map(parse_state_color)
                .transpose()?
                .unwrap_or_else(|| auto_state_color(row_index));
            Ok(StateDefinition {
                bit,
                label: label.to_string(),
                color,
            })
        })
        .collect()
}

fn state_cell<'a>(
    row: &'a [String],
    index: usize,
    column: &str,
    row_index: usize,
) -> Result<&'a str, Box<dyn Error>> {
    row.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("row {} is missing column value: {column}", row_index + 1).into())
}

fn parse_bit(value: &str, row_index: usize) -> Result<u64, Box<dyn Error>> {
    let bit = parse_u64_value(value, row_index, "bit")?;
    if bit == 0 || !bit.is_power_of_two() {
        return Err(format!("row {} column bit must contain one bit", row_index + 1).into());
    }
    Ok(bit)
}

fn parse_state_mask(
    row: &[String],
    index: usize,
    column: &str,
    row_index: usize,
) -> Result<u64, Box<dyn Error>> {
    let value = state_cell(row, index, column, row_index)?.trim();
    parse_u64_value(value, row_index, column)
}

fn parse_u64_value(value: &str, row_index: usize, column: &str) -> Result<u64, Box<dyn Error>> {
    let value = value.trim();
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|hex| u64::from_str_radix(hex, 16))
        .unwrap_or_else(|| value.parse())
        .map_err(|error| {
            format!(
                "failed to parse row {} column {column}: {error}",
                row_index + 1
            )
            .into()
        })
}

fn parse_state_color(value: &str) -> Result<StateColor, Box<dyn Error>> {
    let value = value.trim().trim_start_matches('#');
    if value.is_empty() {
        return Err(String::from("state color must not be empty").into());
    }
    if value.len() != 6 {
        return Err(format!("state color must be #RRGGBB: {value}").into());
    }
    let red = u8::from_str_radix(&value[0..2], 16)?;
    let green = u8::from_str_radix(&value[2..4], 16)?;
    let blue = u8::from_str_radix(&value[4..6], 16)?;
    Ok(StateColor(red, green, blue))
}

fn auto_state_color(index: usize) -> StateColor {
    const COLORS: [StateColor; MAX_STATE_DEFINITIONS] = [
        StateColor(225, 87, 89),
        StateColor(80, 170, 220),
        StateColor(244, 180, 0),
    ];
    COLORS[index]
}

fn save_line_plot_image(
    plot: &PlotData,
    output_path: &Path,
    image_size: (u32, u32),
) -> Result<(), Box<dyn Error>> {
    if !matches!(plot.kind, PlotKind::Line) {
        return Err(String::from("image export supports line plots only").into());
    }

    register_image_font()?;

    let scale = image_scale(image_size);

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let root = BitMapBackend::new(output_path, image_size).into_drawing_area();
    root.fill(&WHITE)?;
    let (chart_area, state_area) = if plot.state_bands.is_some() {
        let state_height = scaled_pixels(180, scale);
        let (chart_area, state_area) = root.split_vertically(image_size.1 - state_height);
        (chart_area, Some(state_area))
    } else {
        (root.clone(), None)
    };

    let mut chart = ChartBuilder::on(&chart_area)
        .margin(scaled_pixels(24, scale))
        .caption(
            plot.title.clone(),
            (IMAGE_FONT_FAMILY, 32.0 * scale).into_font(),
        )
        .x_label_area_size(scaled_pixels(56, scale))
        .y_label_area_size(scaled_pixels(64, scale))
        .build_cartesian_2d(
            plot.x_bounds[0]..plot.x_bounds[1],
            plot.y_bounds[0]..plot.y_bounds[1],
        )?;

    chart
        .configure_mesh()
        .x_desc(plot.x_axis.label.clone())
        .y_desc(plot.y_axis_label.clone())
        .x_label_formatter(&|value| format_axis_value(*value, &plot.x_axis.kind))
        .y_label_formatter(&|value| format_number(*value))
        .label_style((IMAGE_FONT_FAMILY, 18.0 * scale))
        .axis_desc_style((IMAGE_FONT_FAMILY, 20.0 * scale))
        .light_line_style(WHITE.mix(0.15))
        .draw()?;

    for (index, series) in plot.series.iter().enumerate() {
        let color = image_series_color(index);
        let annotation = if let Some(envelope) = image_series_envelope(
            &series.points,
            plot.x_bounds,
            image_size.0.saturating_sub(scaled_pixels(160, scale)),
        ) {
            chart.draw_series(envelope.into_iter().map(|(x, min, max)| {
                PathElement::new(vec![(x, min), (x, max)], color.stroke_width(1))
            }))?
        } else {
            chart.draw_series(LineSeries::new(series.points.iter().copied(), &color))?
        };
        annotation.label(series.name.clone()).legend(move |(x, y)| {
            PathElement::new(vec![(x, y), (x + 24, y)], color.stroke_width(3))
        });
    }

    chart
        .configure_series_labels()
        .label_font((IMAGE_FONT_FAMILY, 18.0 * scale))
        .border_style(BLACK)
        .background_style(WHITE.mix(0.85))
        .draw()?;

    if let (Some(state_bands), Some(state_area)) = (&plot.state_bands, state_area) {
        let labels = state_lane_labels(state_bands).join(" | ");
        let lane_count = state_lane_count(state_bands);
        let mut state_chart = ChartBuilder::on(&state_area)
            .margin(scaled_pixels(16, scale))
            .caption(
                format!("states (top to bottom): {labels}"),
                (IMAGE_FONT_FAMILY, 20.0 * scale).into_font(),
            )
            .x_label_area_size(scaled_pixels(40, scale))
            .y_label_area_size(1)
            .build_cartesian_2d(plot.x_bounds[0]..plot.x_bounds[1], 0.0..lane_count as f64)?;
        state_chart
            .configure_mesh()
            .disable_y_mesh()
            .disable_x_mesh()
            .y_labels(0)
            .x_label_formatter(&|value| format_axis_value(*value, &plot.x_axis.kind))
            .label_style((IMAGE_FONT_FAMILY, 16.0 * scale))
            .draw()?;

        for (index, (track, definition)) in state_lanes(state_bands).into_iter().enumerate() {
            let lane_start = (lane_count - index - 1) as f64;
            let color = RGBColor(definition.color.0, definition.color.1, definition.color.2);
            for (point_index, (start, mask)) in track.points.iter().enumerate() {
                if mask & definition.bit == 0 {
                    continue;
                }
                let end = track
                    .points
                    .get(point_index + 1)
                    .map(|(x, _)| *x)
                    .unwrap_or(plot.x_bounds[1]);
                let start = (*start).max(plot.x_bounds[0]);
                let end = end.min(plot.x_bounds[1]);
                if end > start {
                    state_chart.draw_series(std::iter::once(Rectangle::new(
                        [(start, lane_start), (end, lane_start + 1.0)],
                        color.filled(),
                    )))?;
                }
            }
        }
    }

    root.present()?;
    Ok(())
}

fn image_scale(image_size: (u32, u32)) -> f64 {
    (image_size.0 as f64 / DEFAULT_IMAGE_SIZE.0 as f64
        + image_size.1 as f64 / DEFAULT_IMAGE_SIZE.1 as f64)
        / 2.0
}

fn scaled_pixels(base: u32, scale: f64) -> u32 {
    (base as f64 * scale).round().max(1.0) as u32
}

fn image_series_envelope(
    points: &[(f64, f64)],
    x_bounds: [f64; 2],
    pixel_width: u32,
) -> Option<Vec<(f64, f64, f64)>> {
    if pixel_width == 0 || points.len() <= pixel_width as usize {
        return None;
    }

    let x_range = x_bounds[1] - x_bounds[0];
    if !x_range.is_finite() || x_range <= 0.0 {
        return None;
    }

    let mut bins: Vec<Option<(f64, f64)>> = vec![None; pixel_width as usize];
    for &(x, y) in points {
        if x < x_bounds[0] || x > x_bounds[1] {
            continue;
        }
        let index = (((x - x_bounds[0]) / x_range * pixel_width as f64) as usize)
            .min(pixel_width as usize - 1);
        bins[index] = Some(match bins[index] {
            Some((min, max)) => (min.min(y), max.max(y)),
            None => (y, y),
        });
    }

    Some(
        bins.into_iter()
            .enumerate()
            .filter_map(|(index, range)| {
                range.map(|(min, max)| {
                    let x = x_bounds[0] + (index as f64 + 0.5) / pixel_width as f64 * x_range;
                    (x, min, max)
                })
            })
            .collect(),
    )
}

fn register_image_font() -> Result<(), Box<dyn Error>> {
    static REGISTRATION: OnceLock<Result<(), &'static str>> = OnceLock::new();

    match REGISTRATION.get_or_init(|| {
        register_font(IMAGE_FONT_FAMILY, FontStyle::Normal, IMAGE_FONT)
            .map_err(|_| "failed to load the embedded M PLUS 1 font")
    }) {
        Ok(()) => Ok(()),
        Err(message) => Err((*message).into()),
    }
}

fn image_series_color(index: usize) -> RGBColor {
    const COLORS: [RGBColor; 6] = [
        RGBColor(0, 191, 255),
        RGBColor(255, 193, 7),
        RGBColor(46, 204, 113),
        RGBColor(231, 76, 60),
        RGBColor(155, 89, 182),
        RGBColor(52, 73, 94),
    ];

    COLORS[index % COLORS.len()]
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut out = stdout();
    if let Err(error) = execute!(out, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error.into());
    }
    let backend = CrosstermBackend::new(out);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = execute!(stdout(), LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error.into());
        }
    };
    if let Err(error) = terminal.clear() {
        let _ = restore_terminal(&mut terminal);
        return Err(error.into());
    }
    Ok(terminal)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn Error>> {
    let raw_mode_result = disable_raw_mode();
    let screen_result = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor_result = terminal.show_cursor();

    raw_mode_result?;
    screen_result?;
    cursor_result?;
    Ok(())
}

fn draw_plot(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    plot: &PlotData,
) -> Result<(), Box<dyn Error>> {
    let mut state = PlotState {
        main_viewport: Viewport {
            x_bounds: plot.x_bounds,
            y_bounds: plot.y_bounds,
        },
        derivative_y_bounds: plot.derivative_y_bounds,
        fft_y_bounds: [0.0, 1.0],
        show_derivative: false,
        show_fft: false,
    };

    loop {
        let show_derivative = plot.allow_derivative_panel && state.show_derivative;
        let show_fft = plot.allow_fft_panel && state.show_fft;
        let fft_plot =
            show_fft.then(|| compute_fft_plot_data(&plot.series, state.main_viewport.x_bounds));
        let fft_full_y_bounds = fft_plot.as_ref().map(|plot| plot.y_bounds);
        if let Some(full_bounds) = fft_full_y_bounds {
            state.fft_y_bounds = clamp_or_reset_bounds(state.fft_y_bounds, full_bounds);
        }

        terminal.draw(|frame| {
            let area = frame.area();
            let [content_area, help_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
            let state_height = plot
                .state_bands
                .as_ref()
                .map(|bands| state_lane_count(bands).min(u16::MAX as usize) as u16 + 2)
                .unwrap_or(0);
            let (plot_content_area, state_area) = if state_height == 0 {
                (content_area, None)
            } else {
                let [plot_area, state_area] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(state_height)])
                        .areas(content_area);
                (plot_area, Some(state_area))
            };

            let areas = match (show_derivative, show_fft) {
                (false, false) => {
                    let [main] = Layout::vertical([Constraint::Min(1)]).areas(plot_content_area);
                    vec![main]
                }
                (true, false) | (false, true) => {
                    let [main, secondary] =
                        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .areas(plot_content_area);
                    vec![main, secondary]
                }
                (true, true) => {
                    let [main, secondary, tertiary] = Layout::vertical([
                        Constraint::Percentage(34),
                        Constraint::Percentage(33),
                        Constraint::Percentage(33),
                    ])
                    .areas(plot_content_area);
                    vec![main, secondary, tertiary]
                }
            };
            let help = Paragraph::new(help_text(plot.allow_derivative_panel, plot.allow_fft_panel));
            let mut area_index = 0;
            let main_area = areas[area_index];
            area_index += 1;

            match &plot.kind {
                PlotKind::Line => render_chart(
                    frame,
                    main_area,
                    ChartView {
                        series: &plot.series,
                        x_axis: &plot.x_axis,
                        y_label: &plot.y_axis_label,
                        title: &plot.title,
                        x_bounds: state.main_viewport.x_bounds,
                        y_bounds: state.main_viewport.y_bounds,
                    },
                ),
                PlotKind::HeatmapSource(source) => render_heatmap(
                    frame,
                    main_area,
                    HeatmapView {
                        heatmap_source: source,
                        x_axis: &plot.x_axis,
                        y_label: &plot.y_axis_label,
                        title: &plot.title,
                        x_bounds: state.main_viewport.x_bounds,
                        y_bounds: state.main_viewport.y_bounds,
                    },
                ),
            }

            if show_derivative {
                let derivative_area = areas[area_index];
                area_index += 1;
                render_chart(
                    frame,
                    derivative_area,
                    ChartView {
                        series: &plot.derivative_series,
                        x_axis: &plot.x_axis,
                        y_label: "dy/dx",
                        title: "derivative",
                        x_bounds: state.main_viewport.x_bounds,
                        y_bounds: state.derivative_y_bounds,
                    },
                );
            }

            if let Some(fft_plot) = &fft_plot {
                let fft_area = areas[area_index];
                render_chart(
                    frame,
                    fft_area,
                    ChartView {
                        series: &fft_plot.series,
                        x_axis: &fft_plot.x_axis,
                        y_label: "amplitude",
                        title: "fft",
                        x_bounds: fft_plot.x_bounds,
                        y_bounds: state.fft_y_bounds,
                    },
                );
            }
            if let (Some(state_bands), Some(state_area)) = (&plot.state_bands, state_area) {
                render_state_bands(frame, state_area, state_bands, state.main_viewport.x_bounds);
            }
            frame.render_widget(help, help_area);
        })?;

        match event::read()? {
            Event::Key(key)
                if key.kind == KeyEventKind::Press
                    && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter) =>
            {
                return Ok(());
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('d') if plot.allow_derivative_panel => {
                    state.show_derivative = !state.show_derivative
                }
                KeyCode::Char('f') if plot.allow_fft_panel => state.show_fft = !state.show_fft,
                KeyCode::Left | KeyCode::Char('h') => {
                    pan_bounds(&mut state.main_viewport.x_bounds, plot.x_bounds, -0.1)
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    pan_bounds(&mut state.main_viewport.x_bounds, plot.x_bounds, 0.1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    pan_bounds(&mut state.main_viewport.y_bounds, plot.y_bounds, -0.1);
                    pan_bounds(
                        &mut state.derivative_y_bounds,
                        plot.derivative_y_bounds,
                        -0.1,
                    );
                    if let Some(full_bounds) = fft_full_y_bounds {
                        pan_bounds(&mut state.fft_y_bounds, full_bounds, -0.1);
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    pan_bounds(&mut state.main_viewport.y_bounds, plot.y_bounds, 0.1);
                    pan_bounds(
                        &mut state.derivative_y_bounds,
                        plot.derivative_y_bounds,
                        0.1,
                    );
                    if let Some(full_bounds) = fft_full_y_bounds {
                        pan_bounds(&mut state.fft_y_bounds, full_bounds, 0.1);
                    }
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    zoom_bounds(&mut state.main_viewport.x_bounds, plot.x_bounds, 0.8);
                    zoom_bounds(&mut state.main_viewport.y_bounds, plot.y_bounds, 0.8);
                    zoom_bounds(
                        &mut state.derivative_y_bounds,
                        plot.derivative_y_bounds,
                        0.8,
                    );
                    if let Some(full_bounds) = fft_full_y_bounds {
                        zoom_bounds(&mut state.fft_y_bounds, full_bounds, 0.8);
                    }
                }
                KeyCode::Char('-') => {
                    zoom_bounds(&mut state.main_viewport.x_bounds, plot.x_bounds, 1.25);
                    zoom_bounds(&mut state.main_viewport.y_bounds, plot.y_bounds, 1.25);
                    zoom_bounds(
                        &mut state.derivative_y_bounds,
                        plot.derivative_y_bounds,
                        1.25,
                    );
                    if let Some(full_bounds) = fft_full_y_bounds {
                        zoom_bounds(&mut state.fft_y_bounds, full_bounds, 1.25);
                    }
                }
                KeyCode::Char('x') => {
                    zoom_bounds(&mut state.main_viewport.x_bounds, plot.x_bounds, 0.8)
                }
                KeyCode::Char('X') => {
                    zoom_bounds(&mut state.main_viewport.x_bounds, plot.x_bounds, 1.25)
                }
                KeyCode::Char('y') => {
                    zoom_bounds(&mut state.main_viewport.y_bounds, plot.y_bounds, 0.8);
                    zoom_bounds(
                        &mut state.derivative_y_bounds,
                        plot.derivative_y_bounds,
                        0.8,
                    );
                    if let Some(full_bounds) = fft_full_y_bounds {
                        zoom_bounds(&mut state.fft_y_bounds, full_bounds, 0.8);
                    }
                }
                KeyCode::Char('Y') => {
                    zoom_bounds(&mut state.main_viewport.y_bounds, plot.y_bounds, 1.25);
                    zoom_bounds(
                        &mut state.derivative_y_bounds,
                        plot.derivative_y_bounds,
                        1.25,
                    );
                    if let Some(full_bounds) = fft_full_y_bounds {
                        zoom_bounds(&mut state.fft_y_bounds, full_bounds, 1.25);
                    }
                }
                KeyCode::Char('0') => {
                    state.main_viewport.x_bounds = plot.x_bounds;
                    state.main_viewport.y_bounds = plot.y_bounds;
                    state.derivative_y_bounds = plot.derivative_y_bounds;
                    state.fft_y_bounds = [0.0, 1.0];
                }
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn render_state_bands(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    state_bands: &StateBands,
    x_bounds: [f64; 2],
) {
    let block = Block::default().title("states").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 2 || inner.height == 0 {
        return;
    }

    let label_width = inner.width.min(20);
    let [labels_area, bands_area] =
        Layout::horizontal([Constraint::Length(label_width), Constraint::Min(1)]).areas(inner);
    let labels = state_bands
        .tracks
        .iter()
        .flat_map(|track| {
            state_bands
                .definitions
                .iter()
                .map(move |definition| Line::from(state_lane_label(track, definition)))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(labels), labels_area);

    let width = usize::from(bands_area.width);
    let rows = state_lanes(state_bands)
        .into_iter()
        .map(|(track, definition)| {
            Line::from(
                (0..width)
                    .map(|column| {
                        let ratio = (column as f64 + 0.5) / width.max(1) as f64;
                        let x = x_bounds[0] + (x_bounds[1] - x_bounds[0]) * ratio;
                        let active = state_mask_at(track, x) & definition.bit != 0;
                        let style = if active {
                            Style::default().bg(state_color_for_terminal(definition.color))
                        } else {
                            Style::default()
                        };
                        Span::styled(" ", style)
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rows), bands_area);
}

fn state_mask_at(track: &StateTrack, x: f64) -> u64 {
    track
        .points
        .partition_point(|(point_x, _)| *point_x <= x)
        .checked_sub(1)
        .and_then(|index| track.points.get(index))
        .map(|(_, mask)| *mask)
        .unwrap_or(0)
}

fn state_color_for_terminal(color: StateColor) -> Color {
    Color::Rgb(color.0, color.1, color.2)
}

fn state_lanes(state_bands: &StateBands) -> Vec<(&StateTrack, &StateDefinition)> {
    state_bands
        .tracks
        .iter()
        .flat_map(|track| {
            state_bands
                .definitions
                .iter()
                .map(move |definition| (track, definition))
        })
        .collect()
}

fn state_lane_count(state_bands: &StateBands) -> usize {
    state_bands.tracks.len() * state_bands.definitions.len()
}

fn state_lane_labels(state_bands: &StateBands) -> Vec<String> {
    state_lanes(state_bands)
        .into_iter()
        .map(|(track, definition)| state_lane_label(track, definition))
        .collect()
}

fn state_lane_label(track: &StateTrack, definition: &StateDefinition) -> String {
    match &track.label {
        Some(label) => format!("{label} / {}", definition.label),
        None => definition.label.clone(),
    }
}

fn render_chart(frame: &mut Frame, area: ratatui::layout::Rect, view: ChartView<'_>) {
    let x_labels = axis_labels(view.x_bounds, &view.x_axis.kind);
    let y_labels = axis_labels(view.y_bounds, &AxisKind::Numeric);
    let sampled_series = view
        .series
        .iter()
        .map(|series| visible_points(&series.points, view.x_bounds, area.width))
        .collect::<Vec<_>>();
    let datasets = view
        .series
        .iter()
        .zip(sampled_series.iter())
        .enumerate()
        .map(|(index, (series, sampled_points))| {
            Dataset::default()
                .name(series.name.as_str())
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(SERIES_COLORS[index % SERIES_COLORS.len()]))
                .data(sampled_points)
        })
        .collect::<Vec<_>>();

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(view.title.to_string())
                .borders(Borders::ALL),
        )
        .x_axis(
            Axis::default()
                .title(Line::from(view.x_axis.label.clone()))
                .bounds(view.x_bounds)
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .title(Line::from(view.y_label.to_string()))
                .bounds(view.y_bounds)
                .labels(y_labels),
        );

    frame.render_widget(chart, area);
}

fn render_heatmap(frame: &mut Frame, area: ratatui::layout::Rect, view: HeatmapView<'_>) {
    let block = Block::default()
        .title(view.title.to_string())
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 4 || inner.height < 3 {
        return;
    }

    let [body_area, x_label_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let [y_label_area, heatmap_with_legend_area] =
        Layout::horizontal([Constraint::Length(10), Constraint::Min(1)]).areas(body_area);
    let [heatmap_area, legend_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(10)])
            .areas(heatmap_with_legend_area);

    let x_lines = vec![Line::from(vec![
        Span::raw(format_axis_value(view.x_bounds[0], &view.x_axis.kind)),
        Span::raw(" "),
        Span::raw(view.x_axis.label.clone()),
        Span::raw(" "),
        Span::raw(format_axis_value(view.x_bounds[1], &view.x_axis.kind)),
    ])];
    frame.render_widget(Paragraph::new(x_lines), x_label_area);

    let y_lines = vec![
        Line::from(format_axis_value(view.y_bounds[1], &AxisKind::Numeric)),
        Line::from(view.y_label.to_string()),
        Line::from(format_axis_value(view.y_bounds[0], &AxisKind::Numeric)),
    ];
    frame.render_widget(Paragraph::new(y_lines), y_label_area);

    let render_data = render_heatmap_lines(
        view.heatmap_source,
        view.x_bounds,
        view.y_bounds,
        heatmap_area.width,
        heatmap_area.height,
    );
    frame.render_widget(Paragraph::new(render_data.rows), heatmap_area);
    render_heatmap_legend(frame, legend_area, render_data.value_bounds);
}

fn render_heatmap_lines(
    heatmap_source: &HeatmapSource,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    width: u16,
    height: u16,
) -> HeatmapRenderData {
    let width = usize::from(width.max(1));
    let height = usize::from(height.max(1));
    let intensities = compute_heatmap_grid(heatmap_source, x_bounds, y_bounds, width, height);
    let bounds = axis_bounds_or_default(
        intensities
            .iter()
            .flat_map(|row| row.iter().copied())
            .filter(|value| value.is_finite()),
        [0.0, 1.0],
    );

    let rows = intensities
        .into_iter()
        .map(|row| {
            Line::from(
                row.into_iter()
                    .map(|value| {
                        Span::styled(
                            " ",
                            Style::default()
                                .bg(heatmap_color(normalize_heatmap_value(value, bounds))),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    HeatmapRenderData {
        rows,
        value_bounds: bounds,
    }
}

fn render_heatmap_legend(frame: &mut Frame, area: ratatui::layout::Rect, value_bounds: [f64; 2]) {
    if area.width < 3 || area.height < 4 {
        return;
    }

    let [label_area, bar_area] =
        Layout::horizontal([Constraint::Length(7), Constraint::Length(2)]).areas(area);
    let height = usize::from(bar_area.height.max(1));
    let rows = (0..height)
        .map(|row| {
            let ratio = 1.0 - (row as f64 + 0.5) / height as f64;
            Line::from(vec![Span::styled(
                "  ",
                Style::default().bg(heatmap_color(ratio)),
            )])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rows), bar_area);

    let midpoint = (value_bounds[0] + value_bounds[1]) / 2.0;
    let labels = vec![
        Line::from("Power (dB)"),
        Line::from(format_number(value_bounds[1])),
        Line::from(""),
        Line::from(format_number(midpoint)),
        Line::from(""),
        Line::from(format_number(value_bounds[0])),
    ];
    frame.render_widget(Paragraph::new(labels), label_area);
}

fn normalize_heatmap_value(value: f64, bounds: [f64; 2]) -> f64 {
    let range = bounds[1] - bounds[0];
    if !range.is_finite() || range <= 0.0 {
        0.0
    } else {
        ((value - bounds[0]) / range).clamp(0.0, 1.0)
    }
}

fn heatmap_color(intensity: f64) -> Color {
    let stops = [
        (0.0, (10_u8, 10_u8, 30_u8)),
        (0.25, (0_u8, 90_u8, 160_u8)),
        (0.5, (0_u8, 170_u8, 120_u8)),
        (0.75, (230_u8, 200_u8, 30_u8)),
        (1.0, (250_u8, 80_u8, 20_u8)),
    ];

    for window in stops.windows(2) {
        let (left_pos, left_rgb) = window[0];
        let (right_pos, right_rgb) = window[1];
        if intensity <= right_pos {
            let t = ((intensity - left_pos) / (right_pos - left_pos)).clamp(0.0, 1.0);
            return Color::Rgb(
                lerp_channel(left_rgb.0, right_rgb.0, t),
                lerp_channel(left_rgb.1, right_rgb.1, t),
                lerp_channel(left_rgb.2, right_rgb.2, t),
            );
        }
    }

    Color::Rgb(250, 80, 20)
}

fn lerp_channel(start: u8, end: u8, t: f64) -> u8 {
    (start as f64 + (end as f64 - start as f64) * t).round() as u8
}

fn axis_labels(bounds: [f64; 2], axis_kind: &AxisKind) -> Vec<Line<'static>> {
    let midpoint = (bounds[0] + bounds[1]) / 2.0;
    vec![
        Line::from(format_axis_value(bounds[0], axis_kind)),
        Line::from(format_axis_value(midpoint, axis_kind)),
        Line::from(format_axis_value(bounds[1], axis_kind)),
    ]
}

fn format_axis_value(value: f64, axis_kind: &AxisKind) -> String {
    match axis_kind {
        AxisKind::Numeric => format_number(value),
        AxisKind::Timestamp => {
            format_timestamp_label(value).unwrap_or_else(|| format_number(value))
        }
    }
}

fn format_timestamp_label(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let (seconds, nanos) = split_unix_timestamp(value)?;
    let timestamp = DateTime::<Utc>::from_timestamp(seconds, nanos)?;
    Some(timestamp.format("%m-%d %H:%M:%S").to_string())
}

fn axis_bounds(values: impl Iterator<Item = f64>) -> [f64; 2] {
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for value in values {
        min = min.min(value);
        max = max.max(value);
    }
    if min == max {
        [min - 1.0, max + 1.0]
    } else {
        [min, max]
    }
}

fn axis_bounds_or_default(values: impl Iterator<Item = f64>, default: [f64; 2]) -> [f64; 2] {
    let collected = values.collect::<Vec<_>>();
    if collected.is_empty() {
        default
    } else {
        axis_bounds(collected.into_iter())
    }
}

fn compute_fft_plot_data(series: &[PlotSeries], x_bounds: [f64; 2]) -> PlotData {
    let fft_series = series
        .iter()
        .map(|series| PlotSeries {
            name: format!("fft({})", series.name),
            points: compute_fft_points(&series.points, x_bounds),
        })
        .collect::<Vec<_>>();
    let x_values = fft_series
        .iter()
        .flat_map(|series| series.points.iter().map(|(x, _)| *x))
        .collect::<Vec<_>>();
    let y_values = fft_series
        .iter()
        .flat_map(|series| series.points.iter().map(|(_, y)| *y))
        .collect::<Vec<_>>();

    PlotData {
        kind: PlotKind::Line,
        x_axis: AxisDescriptor {
            label: String::from("frequency"),
            kind: AxisKind::Numeric,
        },
        y_axis_label: String::from("amplitude"),
        title: String::from("fft"),
        series: fft_series,
        derivative_series: Vec::new(),
        x_bounds: axis_bounds_or_default(x_values.into_iter(), [0.0, 1.0]),
        y_bounds: axis_bounds_or_default(y_values.into_iter(), [0.0, 1.0]),
        derivative_y_bounds: [0.0, 1.0],
        state_bands: None,
        allow_derivative_panel: false,
        allow_fft_panel: false,
    }
}

fn compute_fft_points(points: &[(f64, f64)], x_bounds: [f64; 2]) -> Vec<(f64, f64)> {
    let visible = visible_points(points, x_bounds, u16::MAX);
    if visible.len() < 4 {
        return Vec::new();
    }

    let sample_count = visible.len().next_power_of_two().min(1024);
    if sample_count < 4 {
        return Vec::new();
    }

    let start = visible.first().map(|(x, _)| *x).unwrap_or(x_bounds[0]);
    let end = visible.last().map(|(x, _)| *x).unwrap_or(x_bounds[1]);
    let duration = end - start;
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }

    let dt = duration / (sample_count.saturating_sub(1) as f64);
    if !dt.is_finite() || dt <= 0.0 {
        return Vec::new();
    }

    let uniform = resample_uniform(&visible, start, dt, sample_count);
    let mean = uniform.iter().sum::<f64>() / uniform.len() as f64;
    let mut buffer = uniform
        .into_iter()
        .map(|value| Complex::new(value - mean, 0.0))
        .collect::<Vec<_>>();

    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(sample_count);
    fft.process(&mut buffer);

    let scale = sample_count as f64;
    let half = sample_count / 2;
    (0..=half)
        .map(|index| {
            let frequency = index as f64 / (sample_count as f64 * dt);
            let amplitude = if index == 0 || index == half {
                buffer[index].norm() / scale
            } else {
                2.0 * buffer[index].norm() / scale
            };
            (frequency, amplitude)
        })
        .collect()
}

fn infer_sample_spacing(points: &[(f64, f64)]) -> Result<f64, Box<dyn Error>> {
    let intervals = points
        .windows(2)
        .map(|window| {
            let interval = window[1].0 - window[0].0;
            if !interval.is_finite() || interval <= 0.0 {
                Err(String::from(
                    "csv_power_spectrum requires strictly increasing x values",
                ))
            } else {
                Ok(interval)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if intervals.is_empty() {
        return Err(String::from("csv_power_spectrum requires at least two x values").into());
    }
    Ok(intervals.iter().sum::<f64>() / intervals.len() as f64)
}

fn compute_heatmap_grid(
    source: &HeatmapSource,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    width: usize,
    height: usize,
) -> Vec<Vec<f64>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let visible_duration = (x_bounds[1] - x_bounds[0]).max(source.sample_spacing * 4.0);
    let samples_per_column =
        ((visible_duration / source.sample_spacing) / width.max(1) as f64).max(1.0);
    let window_size = ((samples_per_column * 8.0).ceil() as usize)
        .clamp(32, 512)
        .next_power_of_two();
    let half_window_duration = source.sample_spacing * window_size as f64 / 2.0;
    let nyquist_frequency = 0.5 / source.sample_spacing;
    let clamped_y_bounds = [
        y_bounds[0].clamp(0.0, nyquist_frequency),
        y_bounds[1].clamp(0.0, nyquist_frequency),
    ];

    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(window_size);
    let half = window_size / 2;
    let mut rows = vec![vec![0.0; width]; height];

    for column in 0..width {
        let ratio = (column as f64 + 0.5) / width as f64;
        let center_time = x_bounds[0] + (x_bounds[1] - x_bounds[0]) * ratio;
        let window_start = center_time - half_window_duration;
        let samples = resample_uniform(
            &source.points,
            window_start,
            source.sample_spacing,
            window_size,
        );
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let mut buffer = samples
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let hann = 0.5
                    - 0.5
                        * (2.0 * std::f64::consts::PI * index as f64
                            / (window_size.saturating_sub(1).max(1) as f64))
                            .cos();
                Complex::new((value - mean) * hann, 0.0)
            })
            .collect::<Vec<_>>();
        fft.process(&mut buffer);

        for (row_index, row) in rows.iter_mut().enumerate() {
            let frequency_ratio = 1.0 - (row_index as f64 + 0.5) / height as f64;
            let target_frequency =
                clamped_y_bounds[0] + (clamped_y_bounds[1] - clamped_y_bounds[0]) * frequency_ratio;
            let fft_index =
                ((target_frequency * window_size as f64 * source.sample_spacing).round() as usize)
                    .min(half);
            let clamped_index = fft_index.min(buffer.len().saturating_sub(1));
            let amplitude = buffer[clamped_index].norm() / window_size as f64;
            let power = amplitude * amplitude;
            row[column] = 10.0 * (power + 1e-12).log10();
        }
    }

    rows
}

fn help_text(allow_derivative_panel: bool, allow_fft_panel: bool) -> String {
    let mut parts = vec![String::from("q/Esc/Enter: exit")];
    if allow_derivative_panel {
        parts.push(String::from("d: derivative panel"));
    }
    if allow_fft_panel {
        parts.push(String::from("f: fft panel"));
    }
    parts.push(String::from("arrows/hjkl: pan"));
    parts.push(String::from("+/-: zoom"));
    parts.push(String::from("x/X: x zoom"));
    parts.push(String::from("y/Y: y zoom"));
    parts.push(String::from("0: reset"));
    parts.join("  ")
}

fn resample_uniform(points: &[(f64, f64)], start: f64, dt: f64, sample_count: usize) -> Vec<f64> {
    let mut samples = Vec::with_capacity(sample_count);
    let Some(&(first_x, _)) = points.first() else {
        return vec![0.0; sample_count];
    };
    let last_x = points.last().map(|(x, _)| *x).unwrap_or(first_x);
    let boundary_tolerance = (last_x - first_x).abs().max(1.0) * f64::EPSILON * 4.0;
    let mut segment_index = 0;

    for index in 0..sample_count {
        let x = start + dt * index as f64;
        if x < first_x - boundary_tolerance || x > last_x + boundary_tolerance {
            samples.push(0.0);
            continue;
        }

        let x = x.clamp(first_x, last_x);
        while segment_index + 1 < points.len() && points[segment_index + 1].0 < x {
            segment_index += 1;
        }

        let value = if segment_index + 1 >= points.len() {
            points.last().map(|(_, y)| *y).unwrap_or(0.0)
        } else {
            interpolate_between(points[segment_index], points[segment_index + 1], x)
        };
        samples.push(value);
    }

    samples
}

fn interpolate_between((x0, y0): (f64, f64), (x1, y1): (f64, f64), x: f64) -> f64 {
    let dx = x1 - x0;
    if !dx.is_finite() || dx.abs() < f64::EPSILON {
        y0
    } else {
        let t = ((x - x0) / dx).clamp(0.0, 1.0);
        y0 + (y1 - y0) * t
    }
}

fn clamp_or_reset_bounds(bounds: [f64; 2], full_bounds: [f64; 2]) -> [f64; 2] {
    let width = bounds[1] - bounds[0];
    let full_width = full_bounds[1] - full_bounds[0];
    if !width.is_finite() || !full_width.is_finite() || width <= 0.0 || full_width <= 0.0 {
        return full_bounds;
    }
    if bounds[0] < full_bounds[0] || bounds[1] > full_bounds[1] || width > full_width {
        full_bounds
    } else {
        bounds
    }
}

fn compute_derivative_points(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    points
        .windows(2)
        .filter_map(|window| {
            let (x0, y0) = window[0];
            let (x1, y1) = window[1];
            let dx = x1 - x0;
            (dx.is_finite() && dx != 0.0).then_some((x1, (y1 - y0) / dx))
        })
        .collect()
}

fn visible_points(points: &[(f64, f64)], x_bounds: [f64; 2], chart_width: u16) -> Vec<(f64, f64)> {
    let start = points.partition_point(|(x, _)| *x < x_bounds[0]);
    let end = points.partition_point(|(x, _)| *x <= x_bounds[1]);
    let visible = &points[start.min(points.len())..end.min(points.len())];

    if visible.len() <= 2 {
        return visible.to_vec();
    }

    let max_points = usize::from(chart_width.saturating_sub(HORIZONTAL_MARGIN)).max(2);
    if visible.len() <= max_points {
        return visible.to_vec();
    }

    let step = visible.len().div_ceil(max_points);
    let mut sampled = visible.iter().step_by(step).copied().collect::<Vec<_>>();
    if sampled.last() != visible.last() {
        sampled.push(*visible.last().unwrap());
    }
    sampled
}

fn pan_bounds(bounds: &mut [f64; 2], full_bounds: [f64; 2], fraction: f64) {
    let width = bounds[1] - bounds[0];
    let shift = width * fraction;
    bounds[0] += shift;
    bounds[1] += shift;
    clamp_bounds(bounds, full_bounds);
}

fn zoom_bounds(bounds: &mut [f64; 2], full_bounds: [f64; 2], factor: f64) {
    let center = (bounds[0] + bounds[1]) / 2.0;
    let full_width = full_bounds[1] - full_bounds[0];
    let min_width = (full_width * 0.01).max(1e-9);
    let new_width = ((bounds[1] - bounds[0]) * factor).clamp(min_width, full_width);
    bounds[0] = center - new_width / 2.0;
    bounds[1] = center + new_width / 2.0;
    clamp_bounds(bounds, full_bounds);
}

fn clamp_bounds(bounds: &mut [f64; 2], full_bounds: [f64; 2]) {
    let width = bounds[1] - bounds[0];
    let full_width = full_bounds[1] - full_bounds[0];
    if width >= full_width {
        *bounds = full_bounds;
        return;
    }

    if bounds[0] < full_bounds[0] {
        bounds[0] = full_bounds[0];
        bounds[1] = full_bounds[0] + width;
    }
    if bounds[1] > full_bounds[1] {
        bounds[1] = full_bounds[1];
        bounds[0] = full_bounds[1] - width;
    }
}

fn parse_f64_cell(
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

fn parse_timestamp_cell(
    row: &[String],
    index: usize,
    column_name: &str,
    row_index: usize,
) -> Result<f64, Box<dyn Error>> {
    let value = row
        .get(index)
        .ok_or_else(|| format!("row is missing column value: {column_name}"))?;
    parse_timestamp_value(value).ok_or_else(|| {
        format!(
            "failed to parse row {} column {column_name} as timestamp: {value}",
            row_index + 1
        )
        .into()
    })
}

fn parse_timestamp_value(value: &str) -> Option<f64> {
    let value = value.trim();
    if let Ok(unix_seconds) = value.parse::<f64>() {
        return unix_seconds.is_finite().then_some(unix_seconds);
    }

    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|timestamp| timestamp.timestamp_nanos_opt().map(nanos_to_seconds))
        .or_else(|| parse_naive_timestamp(value))
}

fn split_unix_timestamp(value: f64) -> Option<(i64, u32)> {
    if !value.is_finite() {
        return None;
    }

    let mut seconds = value.floor() as i64;
    let mut nanos = ((value - seconds as f64) * 1_000_000_000.0).round() as i64;
    if nanos >= 1_000_000_000 {
        seconds += 1;
        nanos -= 1_000_000_000;
    } else if nanos < 0 {
        seconds -= 1;
        nanos += 1_000_000_000;
    }

    (0..1_000_000_000)
        .contains(&nanos)
        .then_some((seconds, nanos as u32))
}

fn parse_naive_timestamp(value: &str) -> Option<f64> {
    const FORMATS: [&str; 4] = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y/%m/%dT%H:%M:%S%.f",
    ];

    FORMATS.iter().find_map(|format| {
        let naive = NaiveDateTime::parse_from_str(value, format).ok()?;
        let timestamp = Utc.from_utc_datetime(&naive);
        timestamp.timestamp_nanos_opt().map(nanos_to_seconds)
    })
}

fn nanos_to_seconds(nanos: i64) -> f64 {
    nanos as f64 / 1_000_000_000.0
}

fn find_column_index(headers: &[String], name: &str) -> Result<usize, Box<dyn Error>> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format!("column not found: {name}").into())
}

fn format_number(value: f64) -> String {
    format!("{value:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::{temp_path, write_file, write_zst_file};
    use std::fs;

    #[test]
    fn loads_xy_plot_data() {
        let csv = CsvData {
            path: Path::new("input.csv").to_path_buf(),
            headers: vec![String::from("t"), String::from("x"), String::from("v")],
            rows: vec![
                vec![
                    String::from("0.0"),
                    String::from("1.0"),
                    String::from("2.0"),
                ],
                vec![
                    String::from("1.0"),
                    String::from("3.0"),
                    String::from("4.0"),
                ],
            ],
        };

        let plot = load_xy_plot_data(&csv, "t", &[String::from("x"), String::from("v")]).unwrap();

        assert_eq!(plot.series[0].points, vec![(0.0, 1.0), (1.0, 3.0)]);
        assert_eq!(plot.series[1].points, vec![(0.0, 2.0), (1.0, 4.0)]);
        assert_eq!(plot.derivative_series[0].points, vec![(1.0, 2.0)]);
        assert_eq!(plot.derivative_series[1].points, vec![(1.0, 2.0)]);
    }

    #[test]
    fn loads_labeled_series_plot_data() {
        let csv = CsvData {
            path: Path::new("input.csv").to_path_buf(),
            headers: vec![
                String::from("label"),
                String::from("stamp"),
                String::from("signal"),
            ],
            rows: vec![
                vec![
                    String::from("beta"),
                    String::from("2026-01-01T00:00:02Z"),
                    String::from("5.0"),
                ],
                vec![
                    String::from("alpha"),
                    String::from("2026-01-01T00:00:01Z"),
                    String::from("1.5"),
                ],
                vec![
                    String::from("alpha"),
                    String::from("2026-01-01T00:00:03Z"),
                    String::from("2.5"),
                ],
            ],
        };

        let plot = load_labeled_series_plot_data(&csv, "label", "stamp", "signal").unwrap();

        assert_eq!(plot.x_axis.kind, AxisKind::Timestamp);
        assert_eq!(plot.series.len(), 2);
        assert_eq!(plot.series[0].name, "alpha");
        assert_eq!(plot.series[0].points[0].1, 1.5);
        assert!(plot.series[0].points[0].0 < plot.series[0].points[1].0);
        assert_eq!(plot.series[1].name, "beta");
    }

    #[test]
    fn parses_timestamp_values() {
        let rfc3339 = parse_timestamp_value("2026-01-01T12:00:00Z").unwrap();
        let naive = parse_timestamp_value("2026-01-01 12:00:00").unwrap();
        let unix = parse_timestamp_value("1735732800").unwrap();

        assert_eq!(rfc3339, naive);
        assert_eq!(unix, 1_735_732_800.0);
    }

    #[test]
    fn reads_xy_plot_data_from_zst_csv() {
        let path = temp_path("plot.csv.zst");
        write_zst_file(&path, "t,x\n0,1\n1,3\n").unwrap();

        let plot = load_xy_plot_data(&read_csv(&path).unwrap(), "t", &[String::from("x")]).unwrap();

        assert_eq!(plot.series[0].points, vec![(0.0, 1.0), (1.0, 3.0)]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_labeled_series_plot_data_from_plain_csv() {
        let path = temp_path("plot_labels.csv");
        write_file(
            &path,
            "label,stamp,signal\nalpha,2026-01-01T00:00:00Z,1.0\nalpha,2026-01-01T00:00:01Z,2.0\n",
        )
        .unwrap();

        let plot =
            load_labeled_series_plot_data(&read_csv(&path).unwrap(), "label", "stamp", "signal")
                .unwrap();

        assert_eq!(plot.series[0].name, "alpha");
        assert_eq!(plot.series[0].points.len(), 2);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn saves_xy_plot_image() {
        let input = temp_path("plot_image_xy.csv");
        let output = temp_path("plot_image_xy.png");
        write_file(&input, "t,x,v\n0,1,2\n1,3,4\n2,5,6\n").unwrap();

        save_image_xy(
            &input,
            "t",
            &[String::from("x"), String::from("v")],
            &output,
        )
        .unwrap();

        let metadata = fs::metadata(&output).unwrap();
        assert!(metadata.len() > 0);

        fs::remove_file(input).unwrap();
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn saves_xy_plot_image_at_requested_size() {
        let input = temp_path("plot_image_sized.csv");
        let output = temp_path("plot_image_sized.png");
        write_file(&input, "t,signal\n0,1\n1,3\n2,2\n").unwrap();

        save_image_xy_with_state_and_size(
            &input,
            "t",
            &[String::from("signal")],
            None,
            None,
            &output,
            (640, 480),
        )
        .unwrap();

        let png = fs::read(&output).unwrap();
        assert_eq!(&png[16..20], &640u32.to_be_bytes());
        assert_eq!(&png[20..24], &480u32.to_be_bytes());

        fs::remove_file(input).unwrap();
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn image_envelope_preserves_minimum_and_maximum_values() {
        let points = (0..100)
            .map(|index| (index as f64, if index % 2 == 0 { -2.0 } else { 3.0 }))
            .collect::<Vec<_>>();

        let envelope = image_series_envelope(&points, [0.0, 99.0], 10).unwrap();

        assert_eq!(envelope.len(), 10);
        assert!(
            envelope
                .iter()
                .all(|(_, minimum, maximum)| (*minimum, *maximum) == (-2.0, 3.0))
        );
        assert!(image_series_envelope(&points[..10], [0.0, 9.0], 10).is_none());
    }

    #[test]
    fn saves_labeled_series_plot_image() {
        let input = temp_path("plot_image_labels.csv");
        let output = temp_path("nested/plot_image_labels.png");
        write_file(
            &input,
            concat!(
                "label,stamp,signal\n",
                "alpha,2026-01-01T00:00:00Z,1.0\n",
                "beta,2026-01-01T00:00:00Z,2.0\n",
                "alpha,2026-01-01T00:00:01Z,1.5\n",
                "beta,2026-01-01T00:00:01Z,2.5\n",
            ),
        )
        .unwrap();

        save_image_labeled_series(&input, "label", "stamp", "signal", &output).unwrap();

        let metadata = fs::metadata(&output).unwrap();
        assert!(metadata.len() > 0);

        fs::remove_file(input).unwrap();
        fs::remove_file(&output).unwrap();
        fs::remove_dir_all(output.parent().unwrap()).unwrap();
    }

    #[test]
    fn loads_state_bands_with_automatic_and_explicit_colors() {
        let input = temp_path("plot_states.csv");
        let definitions = temp_path("plot_states_definition.csv");
        write_file(&input, "t,signal,state\n0,1,0\n1,2,5\n2,3,4\n").unwrap();
        write_file(&definitions, "bit,label,color\n1,spike,\n4,step,#00ff00\n").unwrap();

        let plot = load_xy_plot_data_with_state(
            &read_csv(&input).unwrap(),
            "t",
            &[String::from("signal")],
            Some(("state", &definitions)),
        )
        .unwrap();
        let state_bands = plot.state_bands.unwrap();

        assert_eq!(state_bands.definitions.len(), 2);
        assert_eq!(state_bands.definitions[0].color, auto_state_color(0));
        assert_eq!(state_bands.definitions[1].color, StateColor(0, 255, 0));
        assert_eq!(state_bands.tracks.len(), 1);
        assert_eq!(state_mask_at(&state_bands.tracks[0], 1.5), 5);
        assert_eq!(state_mask_at(&state_bands.tracks[0], 2.0), 4);

        fs::remove_file(input).unwrap();
        fs::remove_file(definitions).unwrap();
    }

    #[test]
    fn saves_plot_image_with_state_bands() {
        let input = temp_path("plot_image_states.csv");
        let definitions = temp_path("plot_image_states_definition.csv");
        let output = temp_path("plot_image_states.png");
        write_file(&input, "t,signal,state\n0,1,0\n1,2,1\n2,3,3\n").unwrap();
        write_file(&definitions, "bit,label\n1,spike\n2,step\n").unwrap();

        save_image_xy_with_state(
            &input,
            "t",
            &[String::from("signal")],
            Some("state"),
            Some(&definitions),
            &output,
        )
        .unwrap();

        assert!(fs::metadata(&output).unwrap().len() > 0);

        fs::remove_file(input).unwrap();
        fs::remove_file(definitions).unwrap();
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn keeps_state_masks_separate_for_each_labeled_series() {
        let input = temp_path("plot_labeled_states.csv");
        let definitions = temp_path("plot_labeled_states_definition.csv");
        write_file(
            &input,
            concat!(
                "label,stamp,signal,state\n",
                "alpha,0,1,1\n",
                "beta,0,2,2\n",
                "alpha,1,3,0\n",
                "beta,1,4,1\n",
            ),
        )
        .unwrap();
        write_file(&definitions, "bit,label\n1,spike\n2,step\n").unwrap();

        let plot = load_labeled_series_plot_data_with_state(
            &read_csv(&input).unwrap(),
            "label",
            "stamp",
            "signal",
            Some(("state", &definitions)),
        )
        .unwrap();
        let state_bands = plot.state_bands.unwrap();

        assert_eq!(state_bands.tracks.len(), 2);
        assert_eq!(state_bands.tracks[0].label.as_deref(), Some("alpha"));
        assert_eq!(state_mask_at(&state_bands.tracks[0], 0.5), 1);
        assert_eq!(state_bands.tracks[1].label.as_deref(), Some("beta"));
        assert_eq!(state_mask_at(&state_bands.tracks[1], 0.5), 2);

        fs::remove_file(input).unwrap();
        fs::remove_file(definitions).unwrap();
    }

    #[test]
    fn rejects_state_bits_missing_from_definition_csv() {
        let input = temp_path("plot_unknown_state.csv");
        let definitions = temp_path("plot_unknown_state_definition.csv");
        write_file(&input, "t,signal,state\n0,1,2\n1,2,2\n").unwrap();
        write_file(&definitions, "bit,label\n1,spike\n").unwrap();

        let error = load_xy_plot_data_with_state(
            &read_csv(&input).unwrap(),
            "t",
            &[String::from("signal")],
            Some(("state", &definitions)),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("bits missing from the state CSV")
        );
        fs::remove_file(input).unwrap();
        fs::remove_file(definitions).unwrap();
    }

    #[test]
    fn computes_fft_peak_for_sine_wave() {
        let sample_spacing = 0.1;
        let frequency = 10.0 / (128.0 * sample_spacing);
        let points = (0..128)
            .map(|index| {
                let x = index as f64 * sample_spacing;
                let y = (2.0 * std::f64::consts::PI * frequency * x).sin();
                (x, y)
            })
            .collect::<Vec<_>>();

        let fft = compute_fft_points(&points, [0.0, 12.7]);
        let peak = fft
            .iter()
            .skip(1)
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .copied()
            .unwrap();

        assert!(
            (peak.0 - frequency).abs() < 0.01,
            "peak frequency was {}",
            peak.0
        );
        assert!((peak.1 - 1.0).abs() < 0.1, "peak amplitude was {}", peak.1);
    }

    #[test]
    fn rejects_duplicate_timestamps_for_power_spectrum() {
        let csv = CsvData {
            path: Path::new("input.csv").to_path_buf(),
            headers: vec![String::from("t"), String::from("signal")],
            rows: vec![
                vec![String::from("0"), String::from("0")],
                vec![String::from("1"), String::from("1")],
                vec![String::from("1"), String::from("0")],
                vec![String::from("2"), String::from("-1")],
            ],
        };

        let error = load_power_spectrum_plot_data(&csv, "t", "signal").unwrap_err();

        assert!(error.to_string().contains("strictly increasing x values"));
    }

    #[test]
    fn zero_pads_samples_outside_the_data_range() {
        let samples = resample_uniform(&[(0.0, 1.0), (1.0, 1.0)], -1.0, 1.0, 4);

        assert_eq!(samples, vec![0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn rejects_incomplete_state_configuration() {
        let error = state_config_from_args(Some("state"), None).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("--state-column and --state-csv must be specified together")
        );
    }

    #[test]
    fn loads_power_spectrum_plot_data() {
        let csv = CsvData {
            path: Path::new("input.csv").to_path_buf(),
            headers: vec![String::from("t"), String::from("signal")],
            rows: (0..128)
                .map(|index| {
                    let x = index as f64 * 0.1;
                    let y = (2.0 * std::f64::consts::PI * x).sin();
                    vec![x.to_string(), y.to_string()]
                })
                .collect(),
        };

        let plot = load_power_spectrum_plot_data(&csv, "t", "signal").unwrap();

        assert_eq!(plot.x_axis.label, "t");
        assert_eq!(plot.y_axis_label, "frequency");
        assert!(matches!(plot.kind, PlotKind::HeatmapSource(_)));
        assert!(plot.series.is_empty());
        assert!(!plot.allow_derivative_panel);
        assert!(!plot.allow_fft_panel);
    }

    #[test]
    fn computes_heatmap_grid_with_multiple_time_bins() {
        let points = (0..128)
            .map(|index| {
                let x = index as f64 * 0.1;
                let y = if index < 64 {
                    (2.0 * std::f64::consts::PI * 1.0 * x).sin()
                } else {
                    (2.0 * std::f64::consts::PI * 4.0 * x).sin()
                };
                (x, y)
            })
            .collect::<Vec<_>>();

        let heatmap_source = HeatmapSource {
            sample_spacing: infer_sample_spacing(&points).unwrap(),
            points,
        };
        let grid = compute_heatmap_grid(&heatmap_source, [0.0, 12.7], [0.0, 5.0], 24, 12);

        assert_eq!(grid.len(), 12);
        assert_eq!(grid[0].len(), 24);
    }

    #[test]
    fn heatmap_grid_changes_with_y_zoom_range() {
        let points = (0..128)
            .map(|index| {
                let x = index as f64 * 0.1;
                let y = (2.0 * std::f64::consts::PI * 1.0 * x).sin()
                    + 0.3 * (2.0 * std::f64::consts::PI * 4.0 * x).sin();
                (x, y)
            })
            .collect::<Vec<_>>();

        let heatmap_source = HeatmapSource {
            sample_spacing: infer_sample_spacing(&points).unwrap(),
            points,
        };
        let low_band = compute_heatmap_grid(&heatmap_source, [0.0, 12.7], [0.0, 2.0], 16, 8);
        let high_band = compute_heatmap_grid(&heatmap_source, [0.0, 12.7], [3.0, 5.0], 16, 8);

        assert_ne!(low_band, high_band);
    }
}
