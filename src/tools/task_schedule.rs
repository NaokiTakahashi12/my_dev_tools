use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use super::csv_key_diff::{CsvData, read_csv};

const MAX_TASKS: usize = 15;
const MAX_SEARCH_NODES: usize = 1_000_000;
const GANTT_WIDTH: usize = 60;

#[derive(Debug, Clone, PartialEq)]
struct Task {
    id: String,
    priority: u8,
    dependencies: Vec<usize>,
    phases: Vec<Phase>,
}

#[derive(Debug, Clone, PartialEq)]
struct Phase {
    kind: PhaseKind,
    duration_minutes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseKind {
    Manual,
    Auto,
}

impl PhaseKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone)]
struct ScheduleEntry {
    task_index: usize,
    phase_index: usize,
    kind: PhaseKind,
    start_minutes: u64,
    end_minutes: u64,
}

#[derive(Debug, Clone)]
struct ScheduleState {
    time_minutes: u64,
    phase_indices: Vec<usize>,
    auto_end_minutes: Vec<Option<u64>>,
    completion_minutes: Vec<Option<u64>>,
    entries: Vec<ScheduleEntry>,
}

#[derive(Debug, Clone)]
struct Schedule {
    entries: Vec<ScheduleEntry>,
    completion_minutes: Vec<u64>,
    weighted_completion_minutes: u128,
    makespan_minutes: u64,
}

pub fn run(input_path: &Path, output_path: &Path) -> Result<(), Box<dyn Error>> {
    let csv = read_csv(input_path)?;
    let tasks = parse_tasks(&csv)?;
    let schedule = optimize_schedule(&tasks)?;
    write_schedule(output_path, &tasks, &schedule)?;
    print!("{}", render_gantt_chart(&tasks, &schedule));
    println!("schedule: {}", output_path.display());
    Ok(())
}

fn parse_tasks(csv: &CsvData) -> Result<Vec<Task>, Box<dyn Error>> {
    let id_index = find_column_index(&csv.headers, "id")?;
    let priority_index = find_column_index(&csv.headers, "priority")?;
    let dependencies_index = find_column_index(&csv.headers, "dependencies")?;
    let phases_index = find_column_index(&csv.headers, "phases")?;
    if csv.rows.is_empty() {
        return Err(String::from("task CSV requires at least one task").into());
    }
    if csv.rows.len() > MAX_TASKS {
        return Err(format!(
            "task_schedule supports at most {MAX_TASKS} tasks for exact optimization"
        )
        .into());
    }

    let mut raw_tasks = Vec::with_capacity(csv.rows.len());
    for (row_index, row) in csv.rows.iter().enumerate() {
        let id = cell(row, id_index, "id", row_index)?.trim().to_string();
        if id.is_empty() {
            return Err(format!("row {} column id must not be empty", row_index + 1).into());
        }
        let priority = cell(row, priority_index, "priority", row_index)?
            .trim()
            .parse::<u8>()?;
        if !(1..=5).contains(&priority) {
            return Err(format!("row {} priority must be between 1 and 5", row_index + 1).into());
        }
        raw_tasks.push(RawTask {
            id,
            priority,
            dependencies: parse_dependencies(cell(
                row,
                dependencies_index,
                "dependencies",
                row_index,
            )?),
            phases: parse_phases(cell(row, phases_index, "phases", row_index)?, row_index)?,
        });
    }

    raw_tasks.sort_by(|left, right| left.id.cmp(&right.id));
    let ids = raw_tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (task.id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    if ids.len() != raw_tasks.len() {
        return Err(String::from("task CSV contains duplicate id values").into());
    }

    raw_tasks
        .into_iter()
        .enumerate()
        .map(|(index, task)| {
            let dependencies = task
                .dependencies
                .iter()
                .map(|dependency| {
                    let dependency_index = ids.get(dependency).copied().ok_or_else(|| {
                        format!("task {} depends on unknown task {dependency}", task.id)
                    })?;
                    if dependency_index == index {
                        return Err(format!("task {} must not depend on itself", task.id).into());
                    }
                    Ok(dependency_index)
                })
                .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
            if dependencies.iter().copied().collect::<BTreeSet<_>>().len() != dependencies.len() {
                return Err(format!("task {} lists a dependency more than once", task.id).into());
            }
            Ok(Task {
                id: task.id,
                priority: task.priority,
                dependencies,
                phases: task.phases,
            })
        })
        .collect()
}

#[derive(Debug)]
struct RawTask {
    id: String,
    priority: u8,
    dependencies: Vec<String>,
    phases: Vec<Phase>,
}

fn find_column_index(headers: &[String], name: &str) -> Result<usize, Box<dyn Error>> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format!("task CSV is missing required column: {name}").into())
}

fn cell<'a>(
    row: &'a [String],
    index: usize,
    name: &str,
    row_index: usize,
) -> Result<&'a str, Box<dyn Error>> {
    row.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("row {} is missing column {name}", row_index + 1).into())
}

fn parse_dependencies(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_phases(value: &str, row_index: usize) -> Result<Vec<Phase>, Box<dyn Error>> {
    let phases = value
        .split('|')
        .map(str::trim)
        .map(|phase| {
            let (kind, duration) = phase.split_once(':').ok_or_else(|| {
                format!(
                    "row {} phases must use manual:MINUTES (m:MINUTES) or auto:MINUTES (a:MINUTES)",
                    row_index + 1
                )
            })?;
            let kind = match kind.trim() {
                "manual" | "m" => PhaseKind::Manual,
                "auto" | "a" => PhaseKind::Auto,
                other => {
                    return Err(format!(
                        "row {} has unsupported phase type: {other}",
                        row_index + 1
                    ));
                }
            };
            let duration_minutes = duration.trim().parse::<u64>().map_err(|error| {
                format!("row {} has invalid phase duration: {error}", row_index + 1)
            })?;
            if duration_minutes == 0 {
                return Err(format!(
                    "row {} phase duration must be greater than zero",
                    row_index + 1
                ));
            }
            Ok(Phase {
                kind,
                duration_minutes,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error: String| -> Box<dyn Error> { error.into() })?;
    if phases.is_empty() {
        return Err(format!("row {} requires at least one phase", row_index + 1).into());
    }
    if phases.windows(2).any(|pair| pair[0].kind == pair[1].kind) {
        return Err(format!(
            "row {} phases must alternate between manual and auto",
            row_index + 1
        )
        .into());
    }
    Ok(phases)
}

fn optimize_schedule(tasks: &[Task]) -> Result<Schedule, Box<dyn Error>> {
    let mut state = ScheduleState {
        time_minutes: 0,
        phase_indices: vec![0; tasks.len()],
        auto_end_minutes: vec![None; tasks.len()],
        completion_minutes: vec![None; tasks.len()],
        entries: Vec::new(),
    };
    start_automatic_phases(tasks, &mut state)?;

    let mut search_nodes = 0;
    let mut best = None;
    search(tasks, state, &mut search_nodes, &mut best)?;
    best.ok_or_else(|| String::from("task dependencies cannot be scheduled").into())
}

fn search(
    tasks: &[Task],
    state: ScheduleState,
    search_nodes: &mut usize,
    best: &mut Option<Schedule>,
) -> Result<(), Box<dyn Error>> {
    *search_nodes += 1;
    if *search_nodes > MAX_SEARCH_NODES {
        return Err(format!(
            "task schedule optimization exceeded {MAX_SEARCH_NODES} search states"
        )
        .into());
    }
    if state.completion_minutes.iter().all(Option::is_some) {
        let schedule = finish_schedule(tasks, state)?;
        if best
            .as_ref()
            .is_none_or(|current| schedule_is_better(&schedule, current, tasks))
        {
            *best = Some(schedule);
        }
        return Ok(());
    }

    let manual_tasks = available_manual_tasks(tasks, &state);
    for &task_index in &manual_tasks {
        let mut next = state.clone();
        run_manual_phase(tasks, &mut next, task_index)?;
        search(tasks, next, search_nodes, best)?;
    }

    if let Some(next_auto_end) = next_auto_end(&state) {
        let mut next = state;
        advance_to(tasks, &mut next, next_auto_end)?;
        search(tasks, next, search_nodes, best)?;
    } else if manual_tasks.is_empty() {
        return Err(String::from("task dependencies contain a cycle").into());
    }
    Ok(())
}

fn available_manual_tasks(tasks: &[Task], state: &ScheduleState) -> Vec<usize> {
    tasks
        .iter()
        .enumerate()
        .filter_map(|(index, task)| {
            let phase_index = state.phase_indices[index];
            let phase = task.phases.get(phase_index)?;
            (phase.kind == PhaseKind::Manual
                && state.auto_end_minutes[index].is_none()
                && task_can_start(tasks, state, index))
            .then_some(index)
        })
        .collect()
}

fn task_can_start(tasks: &[Task], state: &ScheduleState, task_index: usize) -> bool {
    state.phase_indices[task_index] > 0
        || tasks[task_index]
            .dependencies
            .iter()
            .all(|dependency| state.completion_minutes[*dependency].is_some())
}

fn run_manual_phase(
    tasks: &[Task],
    state: &mut ScheduleState,
    task_index: usize,
) -> Result<(), Box<dyn Error>> {
    let phase_index = state.phase_indices[task_index];
    let phase = &tasks[task_index].phases[phase_index];
    let end_minutes = state
        .time_minutes
        .checked_add(phase.duration_minutes)
        .ok_or_else(|| String::from("schedule time overflow"))?;
    state.entries.push(ScheduleEntry {
        task_index,
        phase_index,
        kind: PhaseKind::Manual,
        start_minutes: state.time_minutes,
        end_minutes,
    });
    advance_automatic_work_until(tasks, state, end_minutes)?;
    state.phase_indices[task_index] += 1;
    mark_completed_if_finished(tasks, state, task_index);
    start_automatic_phases(tasks, state)
}

fn advance_automatic_work_until(
    tasks: &[Task],
    state: &mut ScheduleState,
    end_minutes: u64,
) -> Result<(), Box<dyn Error>> {
    while let Some(next_end) = next_auto_end(state) {
        if next_end > end_minutes {
            break;
        }
        advance_to(tasks, state, next_end)?;
    }
    state.time_minutes = end_minutes;
    Ok(())
}

fn advance_to(
    tasks: &[Task],
    state: &mut ScheduleState,
    time_minutes: u64,
) -> Result<(), Box<dyn Error>> {
    state.time_minutes = time_minutes;
    let completed_automatic_tasks = state
        .auto_end_minutes
        .iter()
        .enumerate()
        .filter_map(|(task_index, auto_end)| {
            (*auto_end == Some(time_minutes)).then_some(task_index)
        })
        .collect::<Vec<_>>();
    for task_index in completed_automatic_tasks {
        state.auto_end_minutes[task_index] = None;
        state.phase_indices[task_index] += 1;
        mark_completed_if_finished(tasks, state, task_index);
    }
    start_automatic_phases(tasks, state)
}

fn start_automatic_phases(tasks: &[Task], state: &mut ScheduleState) -> Result<(), Box<dyn Error>> {
    for (task_index, task) in tasks.iter().enumerate() {
        let phase_index = state.phase_indices[task_index];
        let Some(phase) = task.phases.get(phase_index) else {
            continue;
        };
        if phase.kind != PhaseKind::Auto
            || state.auto_end_minutes[task_index].is_some()
            || !task_can_start(tasks, state, task_index)
        {
            continue;
        }
        let end_minutes = state
            .time_minutes
            .checked_add(phase.duration_minutes)
            .ok_or_else(|| String::from("schedule time overflow"))?;
        state.entries.push(ScheduleEntry {
            task_index,
            phase_index,
            kind: PhaseKind::Auto,
            start_minutes: state.time_minutes,
            end_minutes,
        });
        state.auto_end_minutes[task_index] = Some(end_minutes);
    }
    Ok(())
}

fn mark_completed_if_finished(tasks: &[Task], state: &mut ScheduleState, task_index: usize) {
    if state.phase_indices[task_index] == tasks[task_index].phases.len() {
        state.completion_minutes[task_index] = Some(state.time_minutes);
    }
}

fn next_auto_end(state: &ScheduleState) -> Option<u64> {
    state.auto_end_minutes.iter().flatten().copied().min()
}

fn finish_schedule(tasks: &[Task], state: ScheduleState) -> Result<Schedule, Box<dyn Error>> {
    let completion_minutes = state
        .completion_minutes
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| String::from("schedule contains unfinished tasks"))?;
    let weighted_completion_minutes = tasks
        .iter()
        .zip(&completion_minutes)
        .map(|(task, completion)| priority_weight(task.priority) * u128::from(*completion))
        .sum();
    let makespan_minutes = completion_minutes.iter().copied().max().unwrap_or(0);
    Ok(Schedule {
        entries: state.entries,
        completion_minutes,
        weighted_completion_minutes,
        makespan_minutes,
    })
}

fn priority_weight(priority: u8) -> u128 {
    3_u128.pow(u32::from(priority - 1))
}

fn schedule_is_better(candidate: &Schedule, current: &Schedule, tasks: &[Task]) -> bool {
    (
        candidate.weighted_completion_minutes,
        candidate.makespan_minutes,
    )
        .cmp(&(
            current.weighted_completion_minutes,
            current.makespan_minutes,
        ))
        == std::cmp::Ordering::Less
        || ((
            candidate.weighted_completion_minutes,
            candidate.makespan_minutes,
        ) == (
            current.weighted_completion_minutes,
            current.makespan_minutes,
        ) && manual_task_ids(candidate, tasks) < manual_task_ids(current, tasks))
}

fn manual_task_ids<'a>(schedule: &Schedule, tasks: &'a [Task]) -> Vec<&'a str> {
    schedule
        .entries
        .iter()
        .filter(|entry| entry.kind == PhaseKind::Manual)
        .map(|entry| tasks[entry.task_index].id.as_str())
        .collect()
}

fn write_schedule(
    output_path: &Path,
    tasks: &[Task],
    schedule: &Schedule,
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut entries = schedule.entries.clone();
    entries.sort_by(|left, right| {
        (
            left.start_minutes,
            left.end_minutes,
            left.task_index,
            left.phase_index,
        )
            .cmp(&(
                right.start_minutes,
                right.end_minutes,
                right.task_index,
                right.phase_index,
            ))
    });
    let mut writer = csv::Writer::from_path(output_path)?;
    writer.write_record([
        "record_type",
        "task_id",
        "priority",
        "phase_index",
        "phase_kind",
        "start_minutes",
        "end_minutes",
    ])?;
    for entry in entries {
        writer.write_record([
            "phase".to_string(),
            tasks[entry.task_index].id.clone(),
            tasks[entry.task_index].priority.to_string(),
            entry.phase_index.to_string(),
            entry.kind.as_str().to_string(),
            entry.start_minutes.to_string(),
            entry.end_minutes.to_string(),
        ])?;
    }
    for (task, completion) in tasks.iter().zip(&schedule.completion_minutes) {
        writer.write_record([
            "task_complete".to_string(),
            task.id.clone(),
            task.priority.to_string(),
            String::new(),
            String::new(),
            String::new(),
            completion.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn render_gantt_chart(tasks: &[Task], schedule: &Schedule) -> String {
    render_gantt_chart_with_width(tasks, schedule, GANTT_WIDTH)
}

fn render_gantt_chart_with_width(tasks: &[Task], schedule: &Schedule, width: usize) -> String {
    let width = width.max(1);
    let minutes_per_column = schedule.makespan_minutes.div_ceil(width as u64).max(1);
    let label_width = tasks
        .iter()
        .map(|task| task.id.chars().count())
        .max()
        .unwrap_or(4)
        .clamp(4, 24);
    let mut rendered = format!(
        "\ngantt chart (# manual, = auto)\ntime: 0..{} min, 1 column = {} min\n{:<label_width$} |{}| done\n",
        schedule.makespan_minutes,
        minutes_per_column,
        "task",
        "-".repeat(width),
    );

    for (task_index, task) in tasks.iter().enumerate() {
        let mut row = vec![' '; width];
        for entry in schedule
            .entries
            .iter()
            .filter(|entry| entry.task_index == task_index)
        {
            let start = (entry.start_minutes / minutes_per_column) as usize;
            let end = ((entry.end_minutes - 1) / minutes_per_column) as usize;
            let marker = match entry.kind {
                PhaseKind::Manual => '#',
                PhaseKind::Auto => '=',
            };
            for cell in row
                .iter_mut()
                .take(end.min(width - 1) + 1)
                .skip(start.min(width - 1))
            {
                *cell = marker;
            }
        }
        rendered.push_str(&format!(
            "{:<label_width$} |{}| {} min\n",
            truncate_label(&task.id, label_width),
            row.into_iter().collect::<String>(),
            schedule.completion_minutes[task_index],
        ));
    }
    rendered
}

fn truncate_label(label: &str, width: usize) -> String {
    let mut characters = label.chars();
    let prefix = characters.by_ref().take(width).collect::<String>();
    if characters.next().is_none() || width == 0 {
        return prefix;
    }
    format!("{}~", prefix.chars().take(width - 1).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::{temp_path, write_file};

    fn task(id: &str, priority: u8, phases: &[Phase], dependencies: &[usize]) -> Task {
        Task {
            id: id.to_string(),
            priority,
            dependencies: dependencies.to_vec(),
            phases: phases.to_vec(),
        }
    }

    fn phase(kind: PhaseKind, duration_minutes: u64) -> Phase {
        Phase {
            kind,
            duration_minutes,
        }
    }

    #[test]
    fn parses_tasks_with_dependencies_and_alternating_phases() {
        let csv = CsvData {
            path: Path::new("tasks.csv").to_path_buf(),
            headers: vec![
                String::from("id"),
                String::from("priority"),
                String::from("dependencies"),
                String::from("phases"),
            ],
            rows: vec![
                vec![
                    String::from("deploy"),
                    String::from("5"),
                    String::from("build;test"),
                    String::from("manual:10|auto:20|manual:5"),
                ],
                vec![
                    String::from("build"),
                    String::from("3"),
                    String::new(),
                    String::from("manual:15"),
                ],
                vec![
                    String::from("test"),
                    String::from("4"),
                    String::new(),
                    String::from("auto:10"),
                ],
            ],
        };

        let tasks = parse_tasks(&csv).unwrap();

        assert_eq!(tasks[0].id, "build");
        assert_eq!(tasks[1].id, "deploy");
        assert_eq!(tasks[1].dependencies, vec![0, 2]);
        assert_eq!(tasks[1].phases[1].kind, PhaseKind::Auto);
    }

    #[test]
    fn parses_abbreviated_phase_kinds() {
        let phases = parse_phases("m:10|a:20|m:5", 0).unwrap();

        assert_eq!(
            phases
                .iter()
                .map(|phase| (phase.kind, phase.duration_minutes))
                .collect::<Vec<_>>(),
            vec![
                (PhaseKind::Manual, 10),
                (PhaseKind::Auto, 20),
                (PhaseKind::Manual, 5),
            ]
        );
    }

    #[test]
    fn schedules_manual_work_during_automatic_work() {
        let tasks = vec![
            task("background", 1, &[phase(PhaseKind::Manual, 15)], &[]),
            task(
                "important",
                5,
                &[
                    phase(PhaseKind::Manual, 10),
                    phase(PhaseKind::Auto, 20),
                    phase(PhaseKind::Manual, 10),
                ],
                &[],
            ),
        ];

        let schedule = optimize_schedule(&tasks).unwrap();
        let background = schedule
            .entries
            .iter()
            .find(|entry| tasks[entry.task_index].id == "background")
            .unwrap();
        let automatic = schedule
            .entries
            .iter()
            .find(|entry| {
                tasks[entry.task_index].id == "important" && entry.kind == PhaseKind::Auto
            })
            .unwrap();

        assert_eq!((background.start_minutes, background.end_minutes), (10, 25));
        assert_eq!((automatic.start_minutes, automatic.end_minutes), (10, 30));
        assert_eq!(schedule.completion_minutes, vec![25, 40]);
    }

    #[test]
    fn waits_for_higher_priority_automatic_work() {
        let tasks = vec![
            task("background", 1, &[phase(PhaseKind::Manual, 20)], &[]),
            task(
                "important",
                5,
                &[phase(PhaseKind::Auto, 5), phase(PhaseKind::Manual, 5)],
                &[],
            ),
        ];

        let schedule = optimize_schedule(&tasks).unwrap();
        let important_manual = schedule
            .entries
            .iter()
            .find(|entry| {
                tasks[entry.task_index].id == "important" && entry.kind == PhaseKind::Manual
            })
            .unwrap();

        assert_eq!(
            (important_manual.start_minutes, important_manual.end_minutes),
            (5, 10)
        );
    }

    #[test]
    fn advances_automatic_dependencies_while_manual_work_is_running() {
        let tasks = vec![
            task("auto_source", 1, &[phase(PhaseKind::Auto, 5)], &[]),
            task("dependent", 1, &[phase(PhaseKind::Manual, 5)], &[0]),
            task("important", 5, &[phase(PhaseKind::Manual, 10)], &[]),
        ];

        let schedule = optimize_schedule(&tasks).unwrap();
        let dependent_manual = schedule
            .entries
            .iter()
            .find(|entry| {
                tasks[entry.task_index].id == "dependent" && entry.kind == PhaseKind::Manual
            })
            .unwrap();

        assert_eq!(
            (dependent_manual.start_minutes, dependent_manual.end_minutes),
            (10, 15)
        );
    }

    #[test]
    fn starts_dependents_after_simultaneous_automatic_completions() {
        let tasks = vec![
            task("auto_a", 3, &[phase(PhaseKind::Auto, 5)], &[]),
            task("auto_b", 3, &[phase(PhaseKind::Auto, 5)], &[]),
            task(
                "dependent_auto",
                5,
                &[phase(PhaseKind::Auto, 2), phase(PhaseKind::Manual, 1)],
                &[0, 1],
            ),
            task("dependent_manual", 2, &[phase(PhaseKind::Manual, 1)], &[1]),
        ];

        let schedule = optimize_schedule(&tasks).unwrap();
        let dependent_auto = schedule
            .entries
            .iter()
            .find(|entry| {
                tasks[entry.task_index].id == "dependent_auto" && entry.kind == PhaseKind::Auto
            })
            .unwrap();

        assert_eq!(
            (dependent_auto.start_minutes, dependent_auto.end_minutes),
            (5, 7)
        );
        assert_schedule_is_valid(&tasks, &schedule);
    }

    #[test]
    fn writes_schedule_csv() {
        let input = temp_path("tasks.csv");
        let output = temp_path("nested/schedule.csv");
        write_file(
            &input,
            "id,priority,dependencies,phases\nbuild,3,,manual:10\ntest,5,build,auto:5|manual:5\n",
        )
        .unwrap();

        run(&input, &output).unwrap();

        let csv = fs::read_to_string(&output).unwrap();
        assert!(csv.starts_with("record_type,task_id,priority"));
        assert!(csv.contains("phase,build,3,0,manual,0,10"));
        assert!(csv.contains("phase,test,5,0,auto,10,15"));
        assert!(csv.contains("task_complete,test,5,,,,20"));
        fs::remove_file(input).unwrap();
        fs::remove_dir_all(output.parent().unwrap()).unwrap();
    }

    #[test]
    fn renders_manual_and_automatic_phases_in_gantt_chart() {
        let tasks = vec![task(
            "build",
            3,
            &[
                phase(PhaseKind::Manual, 2),
                phase(PhaseKind::Auto, 3),
                phase(PhaseKind::Manual, 1),
            ],
            &[],
        )];
        let schedule = optimize_schedule(&tasks).unwrap();

        let chart = render_gantt_chart_with_width(&tasks, &schedule, 6);

        assert!(chart.contains("gantt chart (# manual, = auto)"));
        assert!(chart.contains("build |##===#| 6 min"));
    }

    #[test]
    fn enforces_dependencies_and_manual_exclusivity_for_complex_schedule() {
        let tasks = vec![
            task(
                "bootstrap",
                3,
                &[phase(PhaseKind::Manual, 2), phase(PhaseKind::Auto, 5)],
                &[],
            ),
            task("docs", 1, &[phase(PhaseKind::Manual, 4)], &[]),
            task(
                "lint",
                2,
                &[phase(PhaseKind::Auto, 3), phase(PhaseKind::Manual, 2)],
                &[],
            ),
            task(
                "build",
                4,
                &[phase(PhaseKind::Manual, 4), phase(PhaseKind::Auto, 4)],
                &[0],
            ),
            task(
                "test",
                5,
                &[phase(PhaseKind::Auto, 2), phase(PhaseKind::Manual, 3)],
                &[3],
            ),
            task("release", 5, &[phase(PhaseKind::Manual, 1)], &[1, 4]),
        ];

        let schedule = optimize_schedule(&tasks).unwrap();

        assert_schedule_is_valid(&tasks, &schedule);
    }

    #[test]
    fn rejects_circular_dependencies() {
        let tasks = vec![
            task("a", 3, &[phase(PhaseKind::Manual, 1)], &[1]),
            task("b", 3, &[phase(PhaseKind::Auto, 1)], &[0]),
        ];

        let error = optimize_schedule(&tasks).unwrap_err();

        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn rejects_duplicate_dependencies() {
        let csv = CsvData {
            path: Path::new("tasks.csv").to_path_buf(),
            headers: vec![
                String::from("id"),
                String::from("priority"),
                String::from("dependencies"),
                String::from("phases"),
            ],
            rows: vec![
                vec![
                    String::from("a"),
                    String::from("3"),
                    String::new(),
                    String::from("manual:1"),
                ],
                vec![
                    String::from("b"),
                    String::from("3"),
                    String::from("a;a"),
                    String::from("manual:1"),
                ],
            ],
        };

        let error = parse_tasks(&csv).unwrap_err();

        assert!(error.to_string().contains("more than once"));
    }

    #[test]
    fn rejects_unknown_dependencies() {
        let csv = task_csv(vec![vec!["task", "3", "missing", "manual:1"]]);

        let error = parse_tasks(&csv).unwrap_err();

        assert!(error.to_string().contains("unknown task missing"));
    }

    #[test]
    fn rejects_priority_outside_the_supported_range() {
        let csv = task_csv(vec![vec!["task", "6", "", "manual:1"]]);

        let error = parse_tasks(&csv).unwrap_err();

        assert!(error.to_string().contains("between 1 and 5"));
    }

    #[test]
    fn rejects_non_alternating_phases() {
        let csv = task_csv(vec![vec!["task", "3", "", "manual:1|manual:1"]]);

        let error = parse_tasks(&csv).unwrap_err();

        assert!(error.to_string().contains("must alternate"));
    }

    fn task_csv(rows: Vec<Vec<&str>>) -> CsvData {
        CsvData {
            path: Path::new("tasks.csv").to_path_buf(),
            headers: vec![
                String::from("id"),
                String::from("priority"),
                String::from("dependencies"),
                String::from("phases"),
            ],
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(str::to_string).collect())
                .collect(),
        }
    }

    fn assert_schedule_is_valid(tasks: &[Task], schedule: &Schedule) {
        let mut manual_entries = schedule
            .entries
            .iter()
            .filter(|entry| entry.kind == PhaseKind::Manual)
            .collect::<Vec<_>>();
        manual_entries.sort_by_key(|entry| (entry.start_minutes, entry.end_minutes));
        for pair in manual_entries.windows(2) {
            assert!(pair[0].end_minutes <= pair[1].start_minutes);
        }

        for (task_index, task) in tasks.iter().enumerate() {
            let mut entries = schedule
                .entries
                .iter()
                .filter(|entry| entry.task_index == task_index)
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.phase_index);
            assert_eq!(entries.len(), task.phases.len());

            for (phase_index, (entry, phase)) in entries.iter().zip(&task.phases).enumerate() {
                assert_eq!(entry.phase_index, phase_index);
                assert_eq!(entry.kind, phase.kind);
                assert_eq!(
                    entry.end_minutes - entry.start_minutes,
                    phase.duration_minutes
                );
                if phase_index == 0 {
                    for dependency in &task.dependencies {
                        assert!(schedule.completion_minutes[*dependency] <= entry.start_minutes);
                    }
                } else {
                    assert!(entries[phase_index - 1].end_minutes <= entry.start_minutes);
                    if phase.kind == PhaseKind::Auto {
                        assert_eq!(entries[phase_index - 1].end_minutes, entry.start_minutes);
                    }
                }
            }
            assert_eq!(
                schedule.completion_minutes[task_index],
                entries.last().unwrap().end_minutes
            );
        }
    }
}
