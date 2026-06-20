use std::collections::BTreeMap;
use std::error::Error;
use std::io::stdout;
use std::path::Path;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::Line;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph};
use ratatui::{Frame, Terminal};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use super::csv_key_diff::{CsvData, read_csv};

const HORIZONTAL_MARGIN: u16 = 12;
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
    x_axis: AxisDescriptor,
    y_axis_label: String,
    title: String,
    series: Vec<PlotSeries>,
    derivative_series: Vec<PlotSeries>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    derivative_y_bounds: [f64; 2],
    allow_derivative_panel: bool,
    allow_fft_panel: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct PlotConfig {
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
    let csv = read_csv(input_path)?;
    let plot = load_xy_plot_data(&csv, x_column, y_columns)?;
    run_plot(plot)
}

pub fn run_labeled_series(
    input_path: &Path,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let plot = load_labeled_series_plot_data(&csv, label_column, timestamp_column, value_column)?;
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

fn run_plot(plot: PlotData) -> Result<(), Box<dyn Error>> {
    let mut terminal = setup_terminal()?;
    let result = draw_plot(&mut terminal, &plot);
    restore_terminal(&mut terminal)?;
    result
}

fn load_xy_plot_data(
    csv: &CsvData,
    x_column: &str,
    y_columns: &[String],
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

    build_plot_data(
        PlotConfig {
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
    )
}

fn load_labeled_series_plot_data(
    csv: &CsvData,
    label_column: &str,
    timestamp_column: &str,
    value_column: &str,
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

    build_plot_data(
        PlotConfig {
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
    )
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
    let x_bounds = axis_bounds(points.iter().map(|(x, _)| *x));
    let spectrum_points = compute_spectrum_points(&points, x_bounds, SpectrumMode::Power);
    if spectrum_points.is_empty() {
        return Err(String::from("csv_power_spectrum could not compute spectrum").into());
    }

    let spectrum_x_bounds = axis_bounds(spectrum_points.iter().map(|(x, _)| *x));
    let spectrum_y_bounds =
        axis_bounds_or_default(spectrum_points.iter().map(|(_, y)| *y), [0.0, 1.0]);

    Ok(PlotData {
        x_axis: AxisDescriptor {
            label: String::from("frequency"),
            kind: AxisKind::Numeric,
        },
        y_axis_label: String::from("power"),
        title: format!("power({y_column}) vs frequency"),
        series: vec![PlotSeries {
            name: format!("power({y_column})"),
            points: spectrum_points,
        }],
        derivative_series: Vec::new(),
        x_bounds: spectrum_x_bounds,
        y_bounds: spectrum_y_bounds,
        derivative_y_bounds: [0.0, 1.0],
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
        x_axis: config.x_axis,
        y_axis_label: config.y_axis_label,
        title: config.title,
        series,
        derivative_series,
        x_bounds: axis_bounds(x_values.into_iter()),
        y_bounds: axis_bounds(y_values.into_iter()),
        derivative_y_bounds: axis_bounds_or_default(derivative_y_values.into_iter(), [-1.0, 1.0]),
        allow_derivative_panel: config.allow_derivative_panel,
        allow_fft_panel: config.allow_fft_panel,
    })
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
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

            let areas = match (show_derivative, show_fft) {
                (false, false) => {
                    let [main] = Layout::vertical([Constraint::Min(1)]).areas(content_area);
                    vec![main]
                }
                (true, false) | (false, true) => {
                    let [main, secondary] =
                        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .areas(content_area);
                    vec![main, secondary]
                }
                (true, true) => {
                    let [main, secondary, tertiary] = Layout::vertical([
                        Constraint::Percentage(34),
                        Constraint::Percentage(33),
                        Constraint::Percentage(33),
                    ])
                    .areas(content_area);
                    vec![main, secondary, tertiary]
                }
            };
            let help = Paragraph::new(help_text(plot.allow_derivative_panel, plot.allow_fft_panel));
            let mut area_index = 0;
            let main_area = areas[area_index];
            area_index += 1;

            render_chart(
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
            );

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
        allow_derivative_panel: false,
        allow_fft_panel: false,
    }
}

fn compute_fft_points(points: &[(f64, f64)], x_bounds: [f64; 2]) -> Vec<(f64, f64)> {
    compute_spectrum_points(points, x_bounds, SpectrumMode::Amplitude)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpectrumMode {
    Amplitude,
    Power,
}

fn compute_spectrum_points(
    points: &[(f64, f64)],
    x_bounds: [f64; 2],
    spectrum_mode: SpectrumMode,
) -> Vec<(f64, f64)> {
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
            let amplitude = buffer[index].norm() / scale;
            let value = match spectrum_mode {
                SpectrumMode::Amplitude => amplitude,
                SpectrumMode::Power => amplitude * amplitude,
            };
            (frequency, value)
        })
        .collect()
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
    let mut segment_index = 0;

    for index in 0..sample_count {
        let x = start + dt * index as f64;
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
    fn computes_fft_peak_for_sine_wave() {
        let points = (0..128)
            .map(|index| {
                let x = index as f64 * 0.1;
                let y = (2.0 * std::f64::consts::PI * 1.0 * x).sin();
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

        assert!((peak.0 - 1.0).abs() < 0.15, "peak frequency was {}", peak.0);
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

        assert_eq!(plot.x_axis.label, "frequency");
        assert_eq!(plot.y_axis_label, "power");
        assert!(!plot.series[0].points.is_empty());
        assert!(!plot.allow_derivative_panel);
        assert!(!plot.allow_fft_panel);
    }
}
