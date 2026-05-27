use std::error::Error;
use std::io::stdout;
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::Line;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph};

use super::csv_key_diff::read_csv;

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
    x_column: String,
    series: Vec<PlotSeries>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
struct PlotSeries {
    y_column: String,
    points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
struct Viewport {
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
}

pub fn run(input_path: &Path, x_column: &str, y_columns: &[String]) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let plot = load_plot_data(&csv.headers, &csv.rows, x_column, y_columns)?;
    let mut terminal = setup_terminal()?;
    let result = draw_plot(&mut terminal, &plot);
    restore_terminal(&mut terminal)?;
    result
}

fn load_plot_data(
    headers: &[String],
    rows: &[Vec<String>],
    x_column: &str,
    y_columns: &[String],
) -> Result<PlotData, Box<dyn Error>> {
    let x_index = find_column_index(headers, x_column)?;
    if y_columns.is_empty() {
        return Err(String::from("csv_plot requires at least one y column").into());
    }

    let y_indices = y_columns
        .iter()
        .map(|name| find_column_index(headers, name).map(|index| (name.clone(), index)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut series = y_indices
        .iter()
        .map(|(name, _)| PlotSeries {
            y_column: name.clone(),
            points: Vec::with_capacity(rows.len()),
        })
        .collect::<Vec<_>>();

    let mut y_values = Vec::with_capacity(rows.len() * y_indices.len());
    let mut x_values = Vec::with_capacity(rows.len());

    for (row_index, row) in rows.iter().enumerate() {
        let x = parse_cell(row, x_index, x_column, row_index)?;
        x_values.push(x);
        for (series_index, (name, y_index)) in y_indices.iter().enumerate() {
            let y = parse_cell(row, *y_index, name, row_index)?;
            series[series_index].points.push((x, y));
            y_values.push(y);
        }
    }

    if x_values.is_empty() {
        return Err(String::from("csv_plot requires at least one row").into());
    }

    for item in &mut series {
        item.points
            .sort_by(|left, right| left.0.total_cmp(&right.0));
    }

    Ok(PlotData {
        x_column: x_column.to_string(),
        series,
        x_bounds: axis_bounds(x_values.into_iter()),
        y_bounds: axis_bounds(y_values.into_iter()),
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
    let mut viewport = Viewport {
        x_bounds: plot.x_bounds,
        y_bounds: plot.y_bounds,
    };

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let [chart_area, help_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

            let x_labels = axis_labels(viewport.x_bounds);
            let y_labels = axis_labels(viewport.y_bounds);
            let sampled_series = plot
                .series
                .iter()
                .map(|series| visible_points(&series.points, viewport.x_bounds, chart_area.width))
                .collect::<Vec<_>>();
            let datasets = plot
                .series
                .iter()
                .zip(sampled_series.iter())
                .enumerate()
                .map(|(index, (series, sampled_points))| {
                    Dataset::default()
                        .name(series.y_column.as_str())
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(SERIES_COLORS[index % SERIES_COLORS.len()]))
                        .data(sampled_points)
                })
                .collect::<Vec<_>>();
            let chart = Chart::new(datasets)
                .block(
                    Block::default()
                        .title(format!(
                            "{} vs {}",
                            plot.series
                                .iter()
                                .map(|series| series.y_column.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                            plot.x_column
                        ))
                        .borders(Borders::ALL),
                )
                .x_axis(
                    Axis::default()
                        .title(Line::from(plot.x_column.clone()))
                        .bounds(viewport.x_bounds)
                        .labels(x_labels),
                )
                .y_axis(
                    Axis::default()
                        .title(Line::from("y"))
                        .bounds(viewport.y_bounds)
                        .labels(y_labels),
                );
            let help = Paragraph::new(
                "q/Esc/Enter: exit  arrows/hjkl: pan  +/-: zoom  x/X: x zoom  y/Y: y zoom  0: reset",
            );

            frame.render_widget(chart, chart_area);
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
                KeyCode::Left | KeyCode::Char('h') => {
                    pan_bounds(&mut viewport.x_bounds, plot.x_bounds, -0.1)
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    pan_bounds(&mut viewport.x_bounds, plot.x_bounds, 0.1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    pan_bounds(&mut viewport.y_bounds, plot.y_bounds, -0.1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    pan_bounds(&mut viewport.y_bounds, plot.y_bounds, 0.1)
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    zoom_bounds(&mut viewport.x_bounds, plot.x_bounds, 0.8);
                    zoom_bounds(&mut viewport.y_bounds, plot.y_bounds, 0.8);
                }
                KeyCode::Char('-') => {
                    zoom_bounds(&mut viewport.x_bounds, plot.x_bounds, 1.25);
                    zoom_bounds(&mut viewport.y_bounds, plot.y_bounds, 1.25);
                }
                KeyCode::Char('x') => zoom_bounds(&mut viewport.x_bounds, plot.x_bounds, 0.8),
                KeyCode::Char('X') => zoom_bounds(&mut viewport.x_bounds, plot.x_bounds, 1.25),
                KeyCode::Char('y') => zoom_bounds(&mut viewport.y_bounds, plot.y_bounds, 0.8),
                KeyCode::Char('Y') => zoom_bounds(&mut viewport.y_bounds, plot.y_bounds, 1.25),
                KeyCode::Char('0') => {
                    viewport.x_bounds = plot.x_bounds;
                    viewport.y_bounds = plot.y_bounds;
                }
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn axis_labels(bounds: [f64; 2]) -> Vec<Line<'static>> {
    vec![
        Line::from(format_number(bounds[0])),
        Line::from(format_number((bounds[0] + bounds[1]) / 2.0)),
        Line::from(format_number(bounds[1])),
    ]
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

    #[test]
    fn loads_plot_data() {
        let headers = vec![String::from("t"), String::from("x"), String::from("v")];
        let rows = vec![
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
        ];

        let plot = load_plot_data(
            &headers,
            &rows,
            "t",
            &[String::from("x"), String::from("v")],
        )
        .unwrap();

        assert_eq!(plot.series[0].points, vec![(0.0, 1.0), (1.0, 3.0)]);
        assert_eq!(plot.series[1].points, vec![(0.0, 2.0), (1.0, 4.0)]);
        assert_eq!(plot.x_bounds, [0.0, 1.0]);
        assert_eq!(plot.y_bounds, [1.0, 4.0]);
    }

    #[test]
    fn expands_flat_axis_bounds() {
        assert_eq!(axis_bounds([2.0].into_iter()), [1.0, 3.0]);
    }

    #[test]
    fn rejects_non_finite_plot_value() {
        let headers = vec![String::from("t"), String::from("x")];
        let rows = vec![vec![String::from("0.0"), String::from("NaN")]];

        assert!(load_plot_data(&headers, &rows, "t", &[String::from("x")]).is_err());
    }

    #[test]
    fn zooms_bounds_around_center() {
        let mut bounds = [0.0, 10.0];
        zoom_bounds(&mut bounds, [0.0, 10.0], 0.5);
        assert_eq!(bounds, [2.5, 7.5]);
    }

    #[test]
    fn pans_bounds_within_full_range() {
        let mut bounds = [2.0, 6.0];
        pan_bounds(&mut bounds, [0.0, 10.0], 1.0);
        assert_eq!(bounds, [6.0, 10.0]);
    }

    #[test]
    fn samples_only_visible_points() {
        let plot = PlotData {
            x_column: String::from("x"),
            series: vec![PlotSeries {
                y_column: String::from("y"),
                points: vec![(0.0, 0.0), (1.0, 1.0), (2.0, 4.0), (3.0, 9.0)],
            }],
            x_bounds: [0.0, 3.0],
            y_bounds: [0.0, 9.0],
        };

        assert_eq!(
            visible_points(&plot.series[0].points, [1.0, 2.0], 80),
            vec![(1.0, 1.0), (2.0, 4.0)]
        );
    }

    #[test]
    fn downsamples_when_too_many_points() {
        let plot = PlotData {
            x_column: String::from("x"),
            series: vec![PlotSeries {
                y_column: String::from("y"),
                points: (0..100).map(|i| (i as f64, i as f64)).collect(),
            }],
            x_bounds: [0.0, 99.0],
            y_bounds: [0.0, 99.0],
        };

        let sampled = visible_points(&plot.series[0].points, [0.0, 99.0], 20);

        assert!(sampled.len() <= usize::from(20_u16.saturating_sub(HORIZONTAL_MARGIN)) + 1);
        assert_eq!(sampled.first(), Some(&(0.0, 0.0)));
        assert_eq!(sampled.last(), Some(&(99.0, 99.0)));
    }
}
