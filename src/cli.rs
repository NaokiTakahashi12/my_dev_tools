use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use crate::tools::csv_plot;
use crate::tools::{
    csv_anomaly_detect, csv_key_diff, csv_key_diff_extract, csv_label_split, csv_pseudo_diff,
};

#[derive(Debug, PartialEq)]
enum Command {
    KeyDiff {
        left: PathBuf,
        right: PathBuf,
        keys: Vec<String>,
    },
    KeyDiffExtract {
        left: PathBuf,
        right: PathBuf,
        diff: PathBuf,
        output_left: PathBuf,
        output_right: PathBuf,
    },
    PseudoDiff {
        input: PathBuf,
        time_column: String,
        value_column: String,
        time_scale: f64,
        output: Option<PathBuf>,
    },
    AnomalyDetect {
        input: PathBuf,
        x_column: String,
        y_columns: Vec<String>,
    },
    LabelSplit {
        input: PathBuf,
        label_column: String,
        output_dir: Option<PathBuf>,
    },
    Plot {
        input: PathBuf,
        mode: CsvPlotCommand,
    },
    PlotImage {
        input: PathBuf,
        mode: CsvPlotCommand,
        output: PathBuf,
    },
    PowerSpectrum {
        input: PathBuf,
        x_column: String,
        y_column: String,
    },
}

#[derive(Debug, PartialEq)]
enum CsvPlotCommand {
    XY {
        x_column: String,
        y_columns: Vec<String>,
        state: Option<StateOverlayCommand>,
    },
    LabeledSeries {
        label_column: String,
        timestamp_column: String,
        value_column: String,
        state: Option<StateOverlayCommand>,
    },
}

#[derive(Debug, PartialEq)]
struct StateOverlayCommand {
    column: String,
    definitions: PathBuf,
}

pub fn run() -> Result<i32, Box<dyn Error>> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    run_from_args(args)
}

fn run_from_args(args: Vec<OsString>) -> Result<i32, Box<dyn Error>> {
    match parse_args(args)? {
        Command::KeyDiff { left, right, keys } => {
            let left_csv = csv_key_diff::read_csv(&left)?;
            let right_csv = csv_key_diff::read_csv(&right)?;
            let report = csv_key_diff::diff_csv(&left_csv, &right_csv, &keys);
            println!("{}", csv_key_diff::render_diff(&report));
            Ok(report.has_differences().into())
        }
        Command::KeyDiffExtract {
            left,
            right,
            diff,
            output_left,
            output_right,
        } => {
            csv_key_diff_extract::extract_diff_rows_to_files(
                &left,
                &right,
                &diff,
                &output_left,
                &output_right,
            )?;
            Ok(0)
        }
        Command::PseudoDiff {
            input,
            time_column,
            value_column,
            time_scale,
            output,
        } => {
            csv_pseudo_diff::run(
                &input,
                &time_column,
                &value_column,
                time_scale,
                output.as_deref(),
            )?;
            Ok(0)
        }
        Command::AnomalyDetect {
            input,
            x_column,
            y_columns,
        } => {
            csv_anomaly_detect::run(&input, &x_column, &y_columns)?;
            Ok(0)
        }
        Command::LabelSplit {
            input,
            label_column,
            output_dir,
        } => {
            let outputs = csv_label_split::run(&input, &label_column, output_dir.as_deref())?;
            for output in outputs {
                println!("{}", output.display());
            }
            Ok(0)
        }
        Command::Plot { input, mode } => run_csv_plot(&input, &mode),
        Command::PlotImage {
            input,
            mode,
            output,
        } => {
            run_csv_plot_image(&input, &mode, &output)?;
            Ok(0)
        }
        Command::PowerSpectrum {
            input,
            x_column,
            y_column,
        } => {
            csv_plot::run_power_spectrum(&input, &x_column, &y_column)?;
            Ok(0)
        }
    }
}

fn parse_args(args: Vec<OsString>) -> Result<Command, Box<dyn Error>> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err(usage().into());
    };

    match command {
        "csv_key_diff" => parse_csv_key_diff(&args[1..]),
        "csv_key_diff_extract" => parse_csv_key_diff_extract(&args[1..]),
        "csv_pseudo_diff" => parse_csv_pseudo_diff(&args[1..]),
        "csv_anomaly_detect" => parse_csv_anomaly_detect(&args[1..]),
        "csv_label_split" => parse_csv_label_split(&args[1..]),
        "csv_plot" => parse_csv_plot(&args[1..]),
        "csv_plot_image" => parse_csv_plot_image(&args[1..]),
        "csv_power_spectrum" => parse_csv_power_spectrum(&args[1..]),
        _ => Err(format!("unknown command: {command}\n\n{}", usage()).into()),
    }
}

fn parse_csv_key_diff(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    if args.len() < 2 {
        return Err(String::from(
            "csv_key_diff requires LEFT RIGHT and at least one --key VALUE pair",
        )
        .into());
    }

    let left = PathBuf::from(&args[0]);
    let right = PathBuf::from(&args[1]);
    let mut keys = Vec::new();
    let mut index = 2;

    while index < args.len() {
        match args[index].to_str() {
            Some("--key") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --key").into());
                };
                keys.push(value.to_string());
                index += 2;
            }
            Some(other) => {
                return Err(format!("unexpected argument for csv_key_diff: {other}").into());
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    if keys.is_empty() {
        return Err(String::from("csv_key_diff requires at least one --key VALUE pair").into());
    }

    Ok(Command::KeyDiff { left, right, keys })
}

fn parse_csv_key_diff_extract(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    if args.len() < 3 {
        return Err(String::from(
            "csv_key_diff_extract requires LEFT RIGHT DIFF --output_left PATH --output_right PATH",
        )
        .into());
    }

    let left = PathBuf::from(&args[0]);
    let right = PathBuf::from(&args[1]);
    let diff = PathBuf::from(&args[2]);
    let mut output_left = None;
    let mut output_right = None;
    let mut index = 3;

    while index < args.len() {
        match args[index].to_str() {
            Some("--output_left") => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("missing value after --output_left").into());
                };
                output_left = Some(PathBuf::from(value));
                index += 2;
            }
            Some("--output_right") => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("missing value after --output_right").into());
                };
                output_right = Some(PathBuf::from(value));
                index += 2;
            }
            Some(other) => {
                return Err(
                    format!("unexpected argument for csv_key_diff_extract: {other}").into(),
                );
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    let Some(output_left) = output_left else {
        return Err(String::from("csv_key_diff_extract requires --output_left PATH").into());
    };
    let Some(output_right) = output_right else {
        return Err(String::from("csv_key_diff_extract requires --output_right PATH").into());
    };

    Ok(Command::KeyDiffExtract {
        left,
        right,
        diff,
        output_left,
        output_right,
    })
}

fn parse_csv_pseudo_diff(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    if args.is_empty() {
        return Err(String::from(
            "csv_pseudo_diff requires INPUT --time-column NAME --value-column NAME --time-scale VALUE",
        )
        .into());
    }

    let input = PathBuf::from(&args[0]);
    let mut time_column = None;
    let mut value_column = None;
    let mut time_scale = None;
    let mut output = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].to_str() {
            Some("--time-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --time-column").into());
                };
                time_column = Some(value.to_string());
                index += 2;
            }
            Some("--value-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --value-column").into());
                };
                value_column = Some(value.to_string());
                index += 2;
            }
            Some("--time-scale") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --time-scale").into());
                };
                time_scale = Some(value.parse::<f64>()?);
                index += 2;
            }
            Some("--output") => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("missing value after --output").into());
                };
                output = Some(PathBuf::from(value));
                index += 2;
            }
            Some(other) => {
                return Err(format!("unexpected argument for csv_pseudo_diff: {other}").into());
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    let Some(time_column) = time_column else {
        return Err(String::from("csv_pseudo_diff requires --time-column NAME").into());
    };
    let Some(value_column) = value_column else {
        return Err(String::from("csv_pseudo_diff requires --value-column NAME").into());
    };
    let Some(time_scale) = time_scale else {
        return Err(String::from("csv_pseudo_diff requires --time-scale VALUE").into());
    };

    Ok(Command::PseudoDiff {
        input,
        time_column,
        value_column,
        time_scale,
        output,
    })
}

fn parse_csv_anomaly_detect(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    if args.is_empty() {
        return Err(String::from(
            "csv_anomaly_detect requires INPUT --x-column NAME --y-columns NAME[,NAME...]",
        )
        .into());
    }

    let input = PathBuf::from(&args[0]);
    let mut x_column = None;
    let mut y_columns = Vec::new();
    let mut index = 1;

    while index < args.len() {
        match args[index].to_str() {
            Some("--x-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --x-column").into());
                };
                x_column = Some(value.to_string());
                index += 2;
            }
            Some("--y-columns") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --y-columns").into());
                };
                extend_unique_names(&mut y_columns, value);
                index += 2;
            }
            Some(other) => {
                return Err(format!("unexpected argument for csv_anomaly_detect: {other}").into());
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    let Some(x_column) = x_column else {
        return Err(String::from("csv_anomaly_detect requires --x-column NAME").into());
    };
    if y_columns.is_empty() {
        return Err(String::from("csv_anomaly_detect requires --y-columns NAME[,NAME...]").into());
    }

    Ok(Command::AnomalyDetect {
        input,
        x_column,
        y_columns,
    })
}

fn extend_unique_names(target: &mut Vec<String>, value: &str) {
    for name in value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if !target.iter().any(|existing| existing == name) {
            target.push(name.to_string());
        }
    }
}

fn parse_csv_label_split(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    if args.is_empty() {
        return Err(String::from(
            "csv_label_split requires INPUT --label-column NAME [--output-dir PATH]",
        )
        .into());
    }

    let input = PathBuf::from(&args[0]);
    let mut label_column = None;
    let mut output_dir = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].to_str() {
            Some("--label-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --label-column").into());
                };
                label_column = Some(value.to_string());
                index += 2;
            }
            Some("--output-dir") => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("missing value after --output-dir").into());
                };
                output_dir = Some(PathBuf::from(value));
                index += 2;
            }
            Some(other) => {
                return Err(format!("unexpected argument for csv_label_split: {other}").into());
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    let Some(label_column) = label_column else {
        return Err(String::from("csv_label_split requires --label-column NAME").into());
    };

    Ok(Command::LabelSplit {
        input,
        label_column,
        output_dir,
    })
}

fn parse_csv_plot(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    let (input, mode, output) = parse_csv_plot_args(args, false)?;
    debug_assert!(output.is_none());
    Ok(Command::Plot { input, mode })
}

fn parse_csv_plot_image(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    let (input, mode, output) = parse_csv_plot_args(args, true)?;
    let Some(output) = output else {
        return Err(String::from("csv_plot_image requires --output PATH").into());
    };
    Ok(Command::PlotImage {
        input,
        mode,
        output,
    })
}

fn parse_csv_plot_args(
    args: &[OsString],
    require_output: bool,
) -> Result<(PathBuf, CsvPlotCommand, Option<PathBuf>), Box<dyn Error>> {
    if args.is_empty() {
        let message = if require_output {
            "csv_plot_image requires INPUT with either --x-column/--y-columns or --label-column/--timestamp-column/--value-column plus --output PATH"
        } else {
            "csv_plot requires INPUT with either --x-column/--y-columns or --label-column/--timestamp-column/--value-column"
        };
        return Err(String::from(message).into());
    }

    let input = PathBuf::from(&args[0]);
    let mut x_column = None;
    let mut y_columns = Vec::new();
    let mut label_column = None;
    let mut timestamp_column = None;
    let mut value_column = None;
    let mut state_column = None;
    let mut state_csv = None;
    let mut output = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].to_str() {
            Some("--x-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --x-column").into());
                };
                x_column = Some(value.to_string());
                index += 2;
            }
            Some("--y-columns") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --y-columns").into());
                };
                extend_unique_names(&mut y_columns, value);
                index += 2;
            }
            Some("--label-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --label-column").into());
                };
                label_column = Some(value.to_string());
                index += 2;
            }
            Some("--timestamp-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --timestamp-column").into());
                };
                timestamp_column = Some(value.to_string());
                index += 2;
            }
            Some("--value-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --value-column").into());
                };
                value_column = Some(value.to_string());
                index += 2;
            }
            Some("--state-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --state-column").into());
                };
                state_column = Some(value.to_string());
                index += 2;
            }
            Some("--state-csv") => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("missing value after --state-csv").into());
                };
                state_csv = Some(PathBuf::from(value));
                index += 2;
            }
            Some("--output") if require_output => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("missing value after --output").into());
                };
                output = Some(PathBuf::from(value));
                index += 2;
            }
            Some(other) => {
                let command_name = if require_output {
                    "csv_plot_image"
                } else {
                    "csv_plot"
                };
                return Err(format!("unexpected argument for {command_name}: {other}").into());
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    let state = match (state_column, state_csv) {
        (Some(column), Some(definitions)) => Some(StateOverlayCommand {
            column,
            definitions,
        }),
        (None, None) => None,
        _ => {
            let command_name = if require_output {
                "csv_plot_image"
            } else {
                "csv_plot"
            };
            return Err(format!(
                "{command_name} requires --state-column NAME and --state-csv PATH together"
            )
            .into());
        }
    };

    let mode = if let Some(x_column) = x_column {
        if !y_columns.is_empty()
            && label_column.is_none()
            && timestamp_column.is_none()
            && value_column.is_none()
        {
            CsvPlotCommand::XY {
                x_column,
                y_columns,
                state,
            }
        } else {
            let command_name = if require_output {
                "csv_plot_image"
            } else {
                "csv_plot"
            };
            return Err(format!(
                "{command_name} XY mode requires only --x-column NAME --y-columns NAME[,NAME...]{}",
                if require_output { " --output PATH" } else { "" }
            )
            .into());
        }
    } else if y_columns.is_empty() {
        match (label_column, timestamp_column, value_column) {
            (Some(label_column), Some(timestamp_column), Some(value_column)) => {
                CsvPlotCommand::LabeledSeries {
                    label_column,
                    timestamp_column,
                    value_column,
                    state,
                }
            }
            _ => {
                let command_name = if require_output {
                    "csv_plot_image"
                } else {
                    "csv_plot"
                };
                return Err(
                    format!(
                        "{command_name} labeled mode requires --label-column NAME --timestamp-column NAME --value-column NAME{}",
                        if require_output { " --output PATH" } else { "" }
                    )
                    .into(),
                );
            }
        }
    } else {
        let command_name = if require_output {
            "csv_plot_image"
        } else {
            "csv_plot"
        };
        return Err(format!("{command_name} requires --x-column together with --y-columns").into());
    };

    Ok((input, mode, output))
}

fn parse_csv_power_spectrum(args: &[OsString]) -> Result<Command, Box<dyn Error>> {
    if args.is_empty() {
        return Err(String::from(
            "csv_power_spectrum requires INPUT --x-column NAME --y-column NAME",
        )
        .into());
    }

    let input = PathBuf::from(&args[0]);
    let mut x_column = None;
    let mut y_column = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].to_str() {
            Some("--x-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --x-column").into());
                };
                x_column = Some(value.to_string());
                index += 2;
            }
            Some("--y-column") => {
                let Some(value) = args.get(index + 1).and_then(|arg| arg.to_str()) else {
                    return Err(String::from("missing value after --y-column").into());
                };
                y_column = Some(value.to_string());
                index += 2;
            }
            Some(other) => {
                return Err(format!("unexpected argument for csv_power_spectrum: {other}").into());
            }
            None => return Err(String::from("arguments must be valid UTF-8").into()),
        }
    }

    let Some(x_column) = x_column else {
        return Err(String::from("csv_power_spectrum requires --x-column NAME").into());
    };
    let Some(y_column) = y_column else {
        return Err(String::from("csv_power_spectrum requires --y-column NAME").into());
    };

    Ok(Command::PowerSpectrum {
        input,
        x_column,
        y_column,
    })
}

fn run_csv_plot(input: &Path, mode: &CsvPlotCommand) -> Result<i32, Box<dyn Error>> {
    match mode {
        CsvPlotCommand::XY {
            x_column,
            y_columns,
            state,
        } => {
            csv_plot::run_xy_with_state(
                input,
                x_column,
                y_columns,
                state.as_ref().map(|state| state.column.as_str()),
                state.as_ref().map(|state| state.definitions.as_path()),
            )?;
        }
        CsvPlotCommand::LabeledSeries {
            label_column,
            timestamp_column,
            value_column,
            state,
        } => {
            csv_plot::run_labeled_series_with_state(
                input,
                label_column,
                timestamp_column,
                value_column,
                state.as_ref().map(|state| state.column.as_str()),
                state.as_ref().map(|state| state.definitions.as_path()),
            )?;
        }
    }
    Ok(0)
}

fn run_csv_plot_image(
    input: &Path,
    mode: &CsvPlotCommand,
    output: &Path,
) -> Result<(), Box<dyn Error>> {
    match mode {
        CsvPlotCommand::XY {
            x_column,
            y_columns,
            state,
        } => csv_plot::save_image_xy_with_state(
            input,
            x_column,
            y_columns,
            state.as_ref().map(|state| state.column.as_str()),
            state.as_ref().map(|state| state.definitions.as_path()),
            output,
        ),
        CsvPlotCommand::LabeledSeries {
            label_column,
            timestamp_column,
            value_column,
            state,
        } => csv_plot::save_image_labeled_series_with_state(
            input,
            label_column,
            timestamp_column,
            value_column,
            state.as_ref().map(|state| state.column.as_str()),
            state.as_ref().map(|state| state.definitions.as_path()),
            output,
        ),
    }
}

fn usage() -> &'static str {
    concat!(
        "usage:\n",
        "  my_dev_tools csv_anomaly_detect INPUT --x-column NAME --y-columns NAME[,NAME...]\n",
        "  my_dev_tools csv_key_diff LEFT RIGHT --key KEY [--key KEY ...]\n",
        "  my_dev_tools csv_key_diff_extract LEFT RIGHT DIFF --output_left PATH --output_right PATH\n",
        "  my_dev_tools csv_label_split INPUT --label-column NAME [--output-dir PATH]\n",
        "  my_dev_tools csv_power_spectrum INPUT --x-column NAME --y-column NAME\n",
        "  my_dev_tools csv_pseudo_diff INPUT --time-column NAME --value-column NAME --time-scale VALUE [--output PATH]\n",
        "  my_dev_tools csv_plot INPUT --x-column NAME --y-columns NAME[,NAME...] [--state-column NAME --state-csv PATH]\n",
        "  my_dev_tools csv_plot INPUT --label-column NAME --timestamp-column NAME --value-column NAME [--state-column NAME --state-csv PATH]\n",
        "  my_dev_tools csv_plot_image INPUT --x-column NAME --y-columns NAME[,NAME...] --output PATH [--state-column NAME --state-csv PATH]\n",
        "  my_dev_tools csv_plot_image INPUT --label-column NAME --timestamp-column NAME --value-column NAME --output PATH [--state-column NAME --state-csv PATH]\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(|arg| OsString::from(*arg)).collect()
    }

    #[test]
    fn parses_csv_key_diff_command() {
        let command = parse_args(os_args(&[
            "csv_key_diff",
            "left.csv",
            "right.csv",
            "--key",
            "id",
            "--key",
            "sub_id",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::KeyDiff {
                left: PathBuf::from("left.csv"),
                right: PathBuf::from("right.csv"),
                keys: vec![String::from("id"), String::from("sub_id")],
            }
        );
    }

    #[test]
    fn parses_csv_key_diff_extract_command() {
        let command = parse_args(os_args(&[
            "csv_key_diff_extract",
            "left.csv",
            "right.csv",
            "diff.txt",
            "--output_left",
            "left_out.csv",
            "--output_right",
            "right_out.csv",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::KeyDiffExtract {
                left: PathBuf::from("left.csv"),
                right: PathBuf::from("right.csv"),
                diff: PathBuf::from("diff.txt"),
                output_left: PathBuf::from("left_out.csv"),
                output_right: PathBuf::from("right_out.csv"),
            }
        );
    }

    #[test]
    fn parses_csv_pseudo_diff_command() {
        let command = parse_args(os_args(&[
            "csv_pseudo_diff",
            "input.csv",
            "--time-column",
            "t",
            "--value-column",
            "x",
            "--time-scale",
            "0.5",
            "--output",
            "out.csv",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::PseudoDiff {
                input: PathBuf::from("input.csv"),
                time_column: String::from("t"),
                value_column: String::from("x"),
                time_scale: 0.5,
                output: Some(PathBuf::from("out.csv")),
            }
        );
    }

    #[test]
    fn parses_csv_anomaly_detect_command() {
        let command = parse_args(os_args(&[
            "csv_anomaly_detect",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-columns",
            "signal, velocity",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::AnomalyDetect {
                input: PathBuf::from("input.csv"),
                x_column: String::from("stamp"),
                y_columns: vec![String::from("signal"), String::from("velocity")],
            }
        );
    }

    #[test]
    fn parses_csv_label_split_command() {
        let command = parse_args(os_args(&[
            "csv_label_split",
            "input.csv",
            "--label-column",
            "label",
            "--output-dir",
            "out",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::LabelSplit {
                input: PathBuf::from("input.csv"),
                label_column: String::from("label"),
                output_dir: Some(PathBuf::from("out")),
            }
        );
    }

    #[test]
    fn parses_csv_plot_command() {
        let command = parse_args(os_args(&[
            "csv_plot",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-columns",
            "signal, velocity",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Plot {
                input: PathBuf::from("input.csv"),
                mode: CsvPlotCommand::XY {
                    x_column: String::from("stamp"),
                    y_columns: vec![String::from("signal"), String::from("velocity")],
                    state: None,
                },
            }
        );
    }

    #[test]
    fn parses_csv_plot_command_with_repeated_y_columns() {
        let command = parse_args(os_args(&[
            "csv_plot",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-columns",
            "signal",
            "--y-columns",
            "velocity,signal",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Plot {
                input: PathBuf::from("input.csv"),
                mode: CsvPlotCommand::XY {
                    x_column: String::from("stamp"),
                    y_columns: vec![String::from("signal"), String::from("velocity")],
                    state: None,
                },
            }
        );
    }

    #[test]
    fn parses_csv_plot_labeled_series_command() {
        let command = parse_args(os_args(&[
            "csv_plot",
            "input.csv",
            "--label-column",
            "label",
            "--timestamp-column",
            "stamp",
            "--value-column",
            "signal",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Plot {
                input: PathBuf::from("input.csv"),
                mode: CsvPlotCommand::LabeledSeries {
                    label_column: String::from("label"),
                    timestamp_column: String::from("stamp"),
                    value_column: String::from("signal"),
                    state: None,
                },
            }
        );
    }

    #[test]
    fn parses_csv_power_spectrum_command() {
        let command = parse_args(os_args(&[
            "csv_power_spectrum",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-column",
            "signal",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::PowerSpectrum {
                input: PathBuf::from("input.csv"),
                x_column: String::from("stamp"),
                y_column: String::from("signal"),
            }
        );
    }

    #[test]
    fn parses_csv_plot_image_command() {
        let command = parse_args(os_args(&[
            "csv_plot_image",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-columns",
            "signal, velocity",
            "--output",
            "plot.png",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::PlotImage {
                input: PathBuf::from("input.csv"),
                mode: CsvPlotCommand::XY {
                    x_column: String::from("stamp"),
                    y_columns: vec![String::from("signal"), String::from("velocity")],
                    state: None,
                },
                output: PathBuf::from("plot.png"),
            }
        );
    }

    #[test]
    fn parses_csv_plot_image_labeled_series_command() {
        let command = parse_args(os_args(&[
            "csv_plot_image",
            "input.csv",
            "--label-column",
            "label",
            "--timestamp-column",
            "stamp",
            "--value-column",
            "signal",
            "--output",
            "plot.png",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::PlotImage {
                input: PathBuf::from("input.csv"),
                mode: CsvPlotCommand::LabeledSeries {
                    label_column: String::from("label"),
                    timestamp_column: String::from("stamp"),
                    value_column: String::from("signal"),
                    state: None,
                },
                output: PathBuf::from("plot.png"),
            }
        );
    }

    #[test]
    fn parses_csv_plot_state_overlay() {
        let command = parse_args(os_args(&[
            "csv_plot",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-columns",
            "signal",
            "--state-column",
            "anomaly_mask",
            "--state-csv",
            "states.csv",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Plot {
                input: PathBuf::from("input.csv"),
                mode: CsvPlotCommand::XY {
                    x_column: String::from("stamp"),
                    y_columns: vec![String::from("signal")],
                    state: Some(StateOverlayCommand {
                        column: String::from("anomaly_mask"),
                        definitions: PathBuf::from("states.csv"),
                    }),
                },
            }
        );
    }

    #[test]
    fn rejects_incomplete_state_overlay_arguments() {
        let error = parse_args(os_args(&[
            "csv_plot",
            "input.csv",
            "--x-column",
            "stamp",
            "--y-columns",
            "signal",
            "--state-column",
            "anomaly_mask",
        ]))
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("--state-column NAME and --state-csv PATH together")
        );
    }
}
