//! Native total-deadline measurements and comparisons of the measure binary.
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use shapeshifter::puzzle::PuzzleJson;
use wait_timeout::ChildExt;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Deserialize, Serialize)]
struct Record {
    variant: String,
    index: usize,
    level: u32,
    seed: Option<u64>,
    binary_sha256: String,
    puzzle_sha256: String,
    wall_ms: f64,
    budget_ms: f64,
    timed_out: bool,
    returncode: Option<i32>,
    phase_log: Vec<String>,
    solved: bool,
    within_budget: bool,
    #[serde(flatten)]
    details: Map<String, Value>,
}

fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let records: Vec<T> = fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    if records.is_empty() {
        return Err(format!("No records in {}", path.display()).into());
    }
    Ok(records)
}

fn write_record(file: &mut File, record: &Record) -> Result<()> {
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    file.flush()?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn puzzle_hash(puzzle: &Value) -> String {
    sha256(&serde_json::to_vec(puzzle).unwrap())
}

fn verify(puzzle: &PuzzleJson, placements: &[(usize, usize)]) -> Result<()> {
    if placements.len() != puzzle.pieces.len() {
        return Err("Wrong placement count".into());
    }
    let mut board = puzzle.board.clone();
    for (piece, &(row, column)) in puzzle.pieces.iter().zip(placements) {
        if piece.is_empty()
            || piece[0].is_empty()
            || row >= puzzle.rows as usize
            || column >= puzzle.columns as usize
            || piece.len() > puzzle.rows as usize - row
            || piece[0].len() > puzzle.columns as usize - column
        {
            return Err("Placement outside board".into());
        }
        for (r, cells) in piece.iter().enumerate() {
            for (c, &active) in cells.iter().enumerate() {
                if active {
                    board[row + r][column + c] =
                        (board[row + r][column + c] + puzzle.m - 1) % puzzle.m;
                }
            }
        }
    }
    if board.iter().flatten().any(|&value| value != 0) {
        return Err("Incorrect original-order placements".into());
    }
    Ok(())
}

struct Worker(Child);
impl Worker {
    fn kill(&mut self) {
        let _ = self.0.kill();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            self.kill();
        }
        let _ = self.0.wait();
    }
}

fn measure(
    binary: &Path,
    binary_hash: &str,
    puzzle: &Value,
    variant: &str,
    index: usize,
    timeout: Duration,
) -> Result<Record> {
    let input: PuzzleJson = serde_json::from_value(puzzle.clone())?;
    let mut command = Command::new(binary.canonicalize()?);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "RAYON_NUM_THREADS",
            std::thread::available_parallelism()?.get().to_string(),
        );
    let start = Instant::now();
    let mut worker = Worker(command.spawn()?);
    let mut stdin = worker.0.stdin.take().unwrap();
    let mut stdout = worker.0.stdout.take().unwrap();
    let mut stderr = worker.0.stderr.take().unwrap();
    // Drain both pipes while the process runs so logging cannot block its deadline.
    let stdout_reader = std::thread::spawn(move || {
        let mut s = String::new();
        stdout.read_to_string(&mut s).map(|_| s)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut s = String::new();
        stderr.read_to_string(&mut s).map(|_| s)
    });
    let payload = serde_json::to_vec(puzzle)?;
    let input_writer = std::thread::spawn(move || stdin.write_all(&payload));
    let status = worker
        .0
        .wait_timeout(timeout.saturating_sub(start.elapsed()))?;
    let timed_out = status.is_none();
    if timed_out {
        worker.kill();
    }
    let status = worker.0.wait()?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| "stdout reader panicked")??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "stderr reader panicked")??;
    let input_result = input_writer.join().map_err(|_| "stdin writer panicked")?;
    if !timed_out {
        input_result?;
    }
    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut details = Map::new();
    for line in stdout
        .split_inclusive('\n')
        .filter(|line| !line.trim().is_empty())
    {
        if timed_out && !line.ends_with('\n') {
            continue;
        }
        details.extend(serde_json::from_str::<Map<String, Value>>(line)?);
    }
    let solved = details.remove("solved").and_then(|value| value.as_bool());
    if !timed_out && (!status.success() || solved.is_none()) {
        return Err(format!("Measurement worker failed ({status}): {stderr}").into());
    }
    let solved = !timed_out && solved.unwrap_or(false);
    if solved {
        verify(
            &input,
            &serde_json::from_value::<Vec<(usize, usize)>>(details["placements"].clone())?,
        )?;
    }
    #[cfg(unix)]
    let returncode = {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .or_else(|| status.signal().map(|signal| -signal))
    };
    #[cfg(not(unix))]
    let returncode = status.code();
    let record = Record {
        variant: variant.to_owned(),
        index,
        level: input.level,
        seed: puzzle
            .get("seed")
            .map(|seed| serde_json::from_value(seed.clone()))
            .transpose()?
            .flatten(),
        binary_sha256: binary_hash.to_owned(),
        puzzle_sha256: puzzle_hash(puzzle),
        wall_ms,
        budget_ms: timeout.as_secs_f64() * 1000.0,
        timed_out,
        returncode,
        phase_log: stderr.lines().map(str::to_owned).collect(),
        solved,
        within_budget: solved && wall_ms <= timeout.as_secs_f64() * 1000.0,
        details,
    };
    println!(
        "{}",
        json!({"variant": variant, "index": index, "level": record.level, "seed": record.seed,
        "within_budget": record.within_budget, "wall_ms": wall_ms,
        "preparation_ms": record.details.get("preparation_ms"), "search_ms": record.details.get("search_ms"),
        "nodes": record.details.get("nodes")})
    );
    Ok(record)
}

fn cost(record: &Record) -> f64 {
    if record.within_budget {
        record.wall_ms.min(record.budget_ms)
    } else {
        record.budget_ms
    }
}

fn summarize(records: &[Record]) -> Result<Value> {
    let mut variants: BTreeMap<&str, BTreeMap<&str, &Record>> = BTreeMap::new();
    for record in records {
        if variants
            .entry(&record.variant)
            .or_default()
            .insert(&record.puzzle_sha256, record)
            .is_some()
        {
            return Err(format!(
                "Duplicate puzzle for {}: {}",
                record.variant, record.puzzle_sha256
            )
            .into());
        }
    }
    let mut summary = Map::new();
    for (&variant, cases) in &variants {
        let solved = cases.values().filter(|record| record.within_budget).count();
        summary.insert(variant.to_owned(), json!({"total": cases.len(), "solved": solved,
            "failures": cases.len() - solved,
            "capped_mean_s": cases.values().map(|record| cost(record)).sum::<f64>() / cases.len() as f64 / 1000.0}));
    }
    if let (Some(baseline), Some(candidate)) = (variants.get("baseline"), variants.get("candidate"))
    {
        let mut totals = (0, 0.0, 0.0);
        for (key, old) in baseline {
            if let Some(new) = candidate.get(key) {
                if old.budget_ms != new.budget_ms {
                    return Err(format!("Paired time budgets differ for puzzle {key}").into());
                }
                totals.0 += 1;
                totals.1 += cost(old);
                totals.2 += cost(new);
            }
        }
        if totals.0 > 0 {
            summary.insert(
                "paired".to_owned(),
                json!({"total": totals.0,
                "time_reduction_percent": 100.0 * (1.0 - totals.2 / totals.1)}),
            );
        }
    }
    Ok(Value::Object(summary))
}

fn resume(
    path: &Path,
    binary_hash: &str,
    puzzle: &Value,
    variant: &str,
    timeout: Duration,
) -> Result<Option<Record>> {
    if !path.exists() || fs::read_to_string(path)?.trim().is_empty() {
        return Ok(None);
    }
    let mut records = read_jsonl::<Record>(path)?;
    if records.len() != 1 {
        return Err(format!("Expected one record in {}", path.display()).into());
    }
    let record = records.pop().unwrap();
    if record.binary_sha256 != binary_hash
        || record.puzzle_sha256 != puzzle_hash(puzzle)
        || record.variant != variant
        || record.budget_ms != timeout.as_secs_f64() * 1000.0
    {
        return Err(format!(
            "Cannot resume {}: binary, puzzle, variant, or time budget changed",
            path.display()
        )
        .into());
    }
    Ok(Some(record))
}

pub fn main(args: &[String]) -> Result<()> {
    const USAGE: &str = "Usage:\n  bench run --binary PATH --puzzles JSONL --output JSONL --variant NAME [--timeout 120]\n  bench compare --baseline PATH --candidate PATH --puzzles JSONL --output DIRECTORY [--timeout 120]\n  bench summarize RESULTS.jsonl|DIRECTORY\n\nRun and compare use all available CPUs with a total wall-clock deadline.";
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args[0] == "summarize" {
        if args.len() != 2 {
            return Err(USAGE.into());
        }
        let mut path = Path::new(&args[1]).to_path_buf();
        if path.is_dir() {
            path.push("results.jsonl");
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&summarize(&read_jsonl(&path)?)?)?
        );
        return Ok(());
    }
    let allowed: &[&str] = if args[0] == "run" {
        &[
            "--binary",
            "--puzzles",
            "--output",
            "--variant",
            "--timeout",
        ]
    } else {
        &[
            "--baseline",
            "--candidate",
            "--puzzles",
            "--output",
            "--timeout",
        ]
    };
    let mut options = BTreeMap::new();
    let (pairs, remainder) = args[1..].as_chunks::<2>();
    for pair in pairs {
        if !allowed.contains(&pair[0].as_str())
            || options.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(format!("Unknown or duplicate option: {}\n{USAGE}", pair[0]).into());
        }
    }
    if !remainder.is_empty() {
        return Err(USAGE.into());
    }
    let required = |name| {
        options
            .get(name)
            .copied()
            .ok_or_else(|| format!("Missing {name}\n{USAGE}"))
    };
    let timeout_secs: f64 = options.get("--timeout").unwrap_or(&"120").parse()?;
    if !timeout_secs.is_finite() || timeout_secs <= 0.0 {
        return Err("Timeout must be positive and finite".into());
    }
    let timeout = Duration::try_from_secs_f64(timeout_secs)?;
    let puzzles: Vec<Value> = read_jsonl(Path::new(required("--puzzles")?))?;
    let output = Path::new(required("--output")?);
    let mut records = Vec::new();
    if args[0] == "run" {
        let binary = Path::new(required("--binary")?);
        let binary_hash = sha256(&fs::read(binary)?);
        let variant = required("--variant")?;
        let mut file = File::create(output)?;
        for (index, puzzle) in puzzles.iter().enumerate() {
            let record = measure(binary, &binary_hash, puzzle, variant, index, timeout)?;
            write_record(&mut file, &record)?;
            records.push(record);
        }
    } else {
        let mut variants = Vec::new();
        for (variant, option) in [("baseline", "--baseline"), ("candidate", "--candidate")] {
            let binary = Path::new(required(option)?);
            variants.push((variant, binary, sha256(&fs::read(binary)?)));
        }
        fs::create_dir_all(output)?;
        for (index, puzzle) in puzzles.iter().enumerate() {
            fs::write(
                output.join(format!("{index:03}-puzzle.jsonl")),
                format!("{puzzle}\n"),
            )?;
            for offset in 0..2 {
                let (variant, binary, binary_hash) = &variants[(offset + index) % 2];
                let path = output.join(format!("{index:03}-{variant}.jsonl"));
                let mut record = match resume(&path, binary_hash, puzzle, variant, timeout)? {
                    Some(record) => record,
                    None => {
                        let record = measure(binary, binary_hash, puzzle, variant, index, timeout)?;
                        write_record(&mut File::create(&path)?, &record)?;
                        record
                    }
                };
                record.index = index;
                records.push(record);
            }
        }
        let mut file = File::create(output.join("results.jsonl"))?;
        for record in &records {
            write_record(&mut file, record)?;
        }
    }
    println!("{}", serde_json::to_string_pretty(&summarize(&records)?)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "shapeshifter-bench-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn reference() -> (Vec<Value>, Vec<Record>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        (
            read_jsonl(&root.join("benchmarks/puzzles.jsonl")).unwrap(),
            read_jsonl(&root.join("benchmarks/reference.jsonl")).unwrap(),
        )
    }

    #[test]
    fn corpus_hashes_replay_and_summary_match_reference() {
        let (puzzles, records) = reference();
        assert_eq!(puzzles.len(), 32);
        assert_eq!(records.len(), puzzles.len());
        for (puzzle, record) in puzzles.iter().zip(&records) {
            assert_eq!(puzzle_hash(puzzle), record.puzzle_sha256);
            assert_eq!(puzzle["seed"].as_u64(), record.seed);
            verify(
                &serde_json::from_value(puzzle.clone()).unwrap(),
                &serde_json::from_value::<Vec<(usize, usize)>>(
                    record.details["placements"].clone(),
                )
                .unwrap(),
            )
            .unwrap();
        }
        let summary = summarize(&records).unwrap();
        assert_eq!(summary["reference"]["solved"], 32);
        assert!(
            (summary["reference"]["capped_mean_s"].as_f64().unwrap() - 12.0090708888).abs() < 1e-8
        );
    }

    #[test]
    fn summary_charges_failures_the_recorded_budget_and_checks_pairs() {
        let (_, mut records) = reference();
        let mut old = records.remove(0);
        old.variant = "baseline".into();
        old.budget_ms = 1000.0;
        old.wall_ms = 1500.0;
        old.within_budget = false;
        let mut new = old.clone();
        new.variant = "candidate".into();
        new.wall_ms = 500.0;
        new.within_budget = true;
        let summary = summarize(&[old.clone(), new.clone()]).unwrap();
        assert_eq!(summary["baseline"]["failures"], 1);
        assert_eq!(summary["baseline"]["capped_mean_s"], 1.0);
        assert_eq!(summary["paired"]["time_reduction_percent"], 50.0);
        assert!(summarize(&[old.clone(), old.clone()]).is_err());
        new.budget_ms = 2000.0;
        assert!(summarize(&[old, new]).is_err());
    }

    #[test]
    fn resume_requires_matching_binary_puzzle_variant_and_budget() {
        let dir = TempDir::new();
        let path = dir.0.join("result.jsonl");
        let (puzzles, records) = reference();
        let record = &records[0];
        let timeout = Duration::from_secs(120);
        write_record(&mut File::create(&path).unwrap(), record).unwrap();
        assert!(
            resume(
                &path,
                &record.binary_sha256,
                &puzzles[0],
                "reference",
                timeout
            )
            .unwrap()
            .is_some()
        );
        assert!(resume(&path, "different binary", &puzzles[0], "reference", timeout).is_err());
        assert!(
            resume(
                &path,
                &record.binary_sha256,
                &puzzles[1],
                "reference",
                timeout
            )
            .is_err()
        );
        assert!(
            resume(
                &path,
                &record.binary_sha256,
                &puzzles[0],
                "candidate",
                timeout
            )
            .is_err()
        );
        assert!(
            resume(
                &path,
                &record.binary_sha256,
                &puzzles[0],
                "reference",
                Duration::from_secs(1)
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn worker_deadline_includes_preparation_and_rejects_wrong_solutions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let binary = dir.0.join("worker");
        let puzzle = json!({"level": 1, "seed": u64::MAX, "rows": 3, "columns": 3, "m": 2,
            "board": [[1,1,0],[0,0,0],[0,0,0]], "pieces": [[[true]],[[true]]]});
        let run = |script: &str, timeout| {
            fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
            measure(&binary, "fixture", &puzzle, "test", 0, timeout)
        };
        let record = run(
            "cat >/dev/null\nprintf '{\"solved\":true,\"placements\":[[0,0],[0,1]]}\\n'",
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(record.within_budget);
        assert_eq!(record.seed, Some(u64::MAX));
        assert!(
            run(
                "cat >/dev/null\nprintf '{\"solved\":true,\"placements\":[[0,0],[0,0]]}\\n'",
                Duration::from_secs(2)
            )
            .is_err()
        );
        // No READY message: the deadline must cover preparation too.
        let start = Instant::now();
        let record = run("exec sleep 30", Duration::from_millis(50)).unwrap();
        assert!(record.timed_out && !record.solved && !record.within_budget);
        assert_eq!(record.budget_ms, 50.0);
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
