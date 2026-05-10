use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::tools::{csv_key_diff, csv_key_diff_extract};

#[derive(Debug, PartialEq, Eq)]
enum Command {
    CsvKeyDiff {
        left: PathBuf,
        right: PathBuf,
        keys: Vec<String>,
    },
    CsvKeyDiffExtract {
        left: PathBuf,
        right: PathBuf,
        diff: PathBuf,
        output_left: PathBuf,
        output_right: PathBuf,
    },
}

pub fn run() -> Result<i32, Box<dyn Error>> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    run_from_args(args)
}

fn run_from_args(args: Vec<OsString>) -> Result<i32, Box<dyn Error>> {
    match parse_args(args)? {
        Command::CsvKeyDiff { left, right, keys } => {
            let left_csv = csv_key_diff::read_csv(&left)?;
            let right_csv = csv_key_diff::read_csv(&right)?;
            let report = csv_key_diff::diff_csv(&left_csv, &right_csv, &keys);
            println!("{}", csv_key_diff::render_diff(&report));
            Ok(report.has_differences().into())
        }
        Command::CsvKeyDiffExtract {
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
    }
}

fn parse_args(args: Vec<OsString>) -> Result<Command, Box<dyn Error>> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err(usage().into());
    };

    match command {
        "csv_key_diff" => parse_csv_key_diff(&args[1..]),
        "csv_key_diff_extract" => parse_csv_key_diff_extract(&args[1..]),
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

    Ok(Command::CsvKeyDiff { left, right, keys })
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

    Ok(Command::CsvKeyDiffExtract {
        left,
        right,
        diff,
        output_left,
        output_right,
    })
}

fn usage() -> &'static str {
    concat!(
        "usage:\n",
        "  my_dev_tools csv_key_diff LEFT RIGHT --key KEY [--key KEY ...]\n",
        "  my_dev_tools csv_key_diff_extract LEFT RIGHT DIFF --output_left PATH --output_right PATH\n"
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
            Command::CsvKeyDiff {
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
            Command::CsvKeyDiffExtract {
                left: PathBuf::from("left.csv"),
                right: PathBuf::from("right.csv"),
                diff: PathBuf::from("diff.txt"),
                output_left: PathBuf::from("left_out.csv"),
                output_right: PathBuf::from("right_out.csv"),
            }
        );
    }
}
