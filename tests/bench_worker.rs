use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn invoke(binary: &Path, args: &[&str], input: Option<&str>) -> String {
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn batch_benchmarks_run_without_a_separate_solver_binary() {
    let directory = TempDir(
        std::env::temp_dir().join(format!("shapeshifter-bench-worker-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let binary = directory
        .0
        .join(format!("bench{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_bench"), &binary).unwrap();
    let puzzle = r#"{"level":1,"rows":3,"columns":3,"m":2,"board":[[1,1,0],[0,0,0],[0,0,0]],"pieces":[[[true]],[[true]]]}"#;
    for flags in [
        vec!["--worker"],
        vec!["--worker", "--parallel"],
        vec!["--worker", "--exhaustive"],
    ] {
        let result = invoke(&binary, &flags, Some(puzzle));
        let lines: Vec<_> = result.lines().collect();
        assert_eq!(lines.len(), 2, "{result}");
        lines[0]
            .strip_prefix("READY ")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let stats: Vec<_> = lines[1].split_whitespace().collect();
        assert_eq!(stats.len(), 3);
        stats[0].parse::<u64>().unwrap();
        stats[1].parse::<u64>().unwrap();
        assert_eq!(stats[2], "true");
    }
    let result = invoke(
        &binary,
        &["simulated", "1", "1", "--games-per", "1", "--timeout", "5"],
        None,
    );
    assert!(
        result.contains("1 ok, 0 fail, 0 timeout, 0 error"),
        "{result}"
    );
    let history = directory.0.join("puzzles.jsonl");
    std::fs::write(&history, format!("{puzzle}\n")).unwrap();
    let result = invoke(
        &binary,
        &[
            "historical",
            history.to_str().unwrap(),
            "--parallel",
            "--timeout",
            "5",
        ],
        None,
    );
    assert!(
        result.contains("1 ok, 0 fail, 0 timeout, 0 error"),
        "{result}"
    );

    let mut impossible: serde_json::Value = serde_json::from_str(puzzle).unwrap();
    impossible["pieces"].as_array_mut().unwrap().pop();
    std::fs::write(&history, format!("{puzzle}\n{impossible}\n")).unwrap();
    let result = invoke(
        &binary,
        &["historical", history.to_str().unwrap(), "--timeout", "5"],
        None,
    );
    assert!(
        result.contains("1 ok, 1 fail, 0 timeout, 0 error"),
        "{result}"
    );
}
