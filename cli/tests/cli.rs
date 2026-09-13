//! End-to-end tests for the `monotone` binary.
//!
//! Tests that need DynamoDB run against `AWS_ENDPOINT_URL` (normally DynamoDB
//! Local) and skip with a notice when it is unset, unless
//! `MONOTONE_REQUIRE_INTEGRATION` is set. Each test uses its own table.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};

fn endpoint() -> Option<String> {
    match std::env::var("AWS_ENDPOINT_URL") {
        Ok(url) => Some(url),
        Err(_) => {
            assert!(
                std::env::var_os("MONOTONE_REQUIRE_INTEGRATION").is_none(),
                "MONOTONE_REQUIRE_INTEGRATION is set but AWS_ENDPOINT_URL is not"
            );
            eprintln!("skipping CLI integration test: AWS_ENDPOINT_URL is not set");
            None
        }
    }
}

/// A command isolated from the developer's AWS profile and credentials.
fn bare() -> Command {
    let mut cmd = Command::cargo_bin("monotone").unwrap();
    cmd.env_remove("RUST_LOG")
        .env_remove("AWS_PROFILE")
        .env("AWS_CONFIG_FILE", "/nonexistent/monotone-test-config")
        .env(
            "AWS_SHARED_CREDENTIALS_FILE",
            "/nonexistent/monotone-test-credentials",
        )
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .env("AWS_ACCESS_KEY_ID", "local")
        .env("AWS_SECRET_ACCESS_KEY", "local");
    cmd
}

/// A command pointed at the test endpoint.
fn cli(endpoint: &str) -> Command {
    let mut cmd = bare();
    cmd.env("AWS_ENDPOINT_URL", endpoint);
    cmd
}

fn unique_table() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "cli-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

// ---- no DynamoDB needed ----

#[test]
fn version_flag_prints_crate_version() {
    bare()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("monotone {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_names_program_and_subcommands() {
    bare()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: monotone"))
        .stdout(predicate::str::contains("counter"))
        .stdout(predicate::str::contains("queue"));
}

#[test]
fn missing_id_prints_error_and_help_and_exits_1() {
    bare()
        .args(["counter", "get"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("Missing required argument id"))
        .stderr(predicate::str::contains("Usage: monotone"));
}

#[test]
fn missing_process_on_join_exits_1() {
    bare()
        .args(["-i", "q", "queue", "join"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "Missing required argument process",
        ))
        .stderr(predicate::str::contains("Usage: monotone"));
    for op in ["get", "leave"] {
        bare()
            .args(["-i", "q", "queue", op])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(
                "Missing required argument process",
            ));
    }
}

#[test]
fn malformed_tag_fails_before_contacting_dynamodb() {
    // A closed port proves no request is attempted: the error is about the tag.
    bare()
        .env("AWS_ENDPOINT_URL", "http://127.0.0.1:9")
        .args(["-i", "q", "queue", "-p", "a", "join", "--tag", "novalue"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("invalid tag: novalue"))
        .stderr(predicate::str::contains("dispatch").not());
}

#[test]
fn no_subcommand_at_either_level_exits_1() {
    for args in [
        vec!["-i", "x"],
        vec!["-i", "x", "counter"],
        vec!["-i", "x", "queue"],
    ] {
        bare()
            .args(&args)
            .assert()
            .code(1)
            .stderr(predicate::str::contains("No subcommand provided"))
            .stderr(predicate::str::contains("Usage: monotone"));
    }
}

#[test]
fn unrecognised_subcommand_exits_1_with_help() {
    for args in [vec!["-i", "x", "frob"], vec!["-i", "x", "counter", "frob"]] {
        bare()
            .args(&args)
            .assert()
            .code(1)
            .stderr(predicate::str::contains("unrecognized subcommand"))
            .stderr(predicate::str::contains("Usage: monotone"));
    }
}

#[test]
fn tag_has_no_short_form() {
    bare()
        .args(["queue", "join", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--tag <KEY=VALUE>"))
        .stdout(predicate::str::contains("-t, --tag").not());
}

#[test]
fn unreachable_endpoint_exits_1_with_connection_error() {
    bare()
        .env("AWS_ENDPOINT_URL", "http://127.0.0.1:9")
        .env("AWS_MAX_ATTEMPTS", "1")
        .args(["-i", "x", "counter", "get"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with("error: "))
        .stderr(predicate::str::contains("dispatch failure"))
        .stderr(predicate::str::contains("panicked").not());
}

// ---- against DynamoDB ----

fn counter_json(table: &str, value: u64) -> String {
    format!(
        r#"{{
  "id": "mycounter",
  "value": {value},
  "region": "eu-west-1",
  "table": "{table}"
}}
"#
    )
}

#[test]
fn counter_get_next_rm_output() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();

    cli(&ep)
        .args(["-t", &table, "-i", "mycounter", "counter", "get"])
        .assert()
        .success()
        .stderr("")
        .stdout(counter_json(&table, 0));

    cli(&ep)
        .args(["-t", &table, "-i", "mycounter", "counter", "next"])
        .assert()
        .success()
        .stdout(counter_json(&table, 1));

    cli(&ep)
        .args(["-t", &table, "-i", "mycounter", "counter", "rm"])
        .assert()
        .success()
        .stdout("");

    let out = cli(&ep)
        .args(["-t", &table, "-i", "mycounter", "counter", "get"])
        .output()
        .unwrap();
    assert_eq!(stdout_json(&out)["value"], 0);
}

#[test]
fn queue_join_leave_list_get_output() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    let q = |args: &[&str]| {
        let mut cmd = cli(&ep);
        cmd.args(["-t", &table, "-i", "myqueue", "queue"])
            .args(args);
        cmd
    };
    let head = |token: u64| {
        format!(
            r#"  "id": "myqueue",
  "region": "eu-west-1",
  "table": "{table}",
  "fencing_token": {token}"#
        )
    };
    let foo = r#"{
    "process_id": "foo",
    "counter": 1,
    "position": 0,
    "tags": {}
  }"#;

    q(&["-p", "foo", "join"])
        .assert()
        .success()
        .stderr("")
        .stdout(format!("{{\n{},\n  \"ticket\": {foo}\n}}\n", head(1)));

    q(&["-p", "bar", "join", "--tag", "role=zk", "--tag", "rack=a"])
        .assert()
        .success()
        .stdout(format!(
            r#"{{
{},
  "ticket": {{
    "process_id": "bar",
    "counter": 2,
    "position": 1,
    "tags": {{
      "rack": "a",
      "role": "zk"
    }}
  }}
}}
"#,
            head(2)
        ));

    q(&["list"]).assert().success().stdout(format!(
        r#"{{
{},
  "tickets": [
    {{
      "process_id": "foo",
      "counter": 1,
      "position": 0,
      "tags": {{}}
    }},
    {{
      "process_id": "bar",
      "counter": 2,
      "position": 1,
      "tags": {{
        "rack": "a",
        "role": "zk"
      }}
    }}
  ]
}}
"#,
        head(2)
    ));

    q(&["-p", "foo", "leave"])
        .assert()
        .success()
        .stdout(format!("{{\n{}\n}}\n", head(3)));

    let out = q(&["-p", "bar", "get"]).output().unwrap();
    let v = stdout_json(&out);
    assert_eq!(v["fencing_token"], 3);
    assert_eq!(
        v["ticket"],
        json!({"process_id": "bar", "counter": 2, "position": 0, "tags": {"rack": "a", "role": "zk"}})
    );

    q(&["rm"]).assert().success().stdout("");
    let out = q(&["list"]).output().unwrap();
    assert_eq!(stdout_json(&out)["fencing_token"], 0);
    assert_eq!(stdout_json(&out)["tickets"], json!([]));
}

#[test]
fn join_output_pipes_to_a_single_counter() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    let out = cli(&ep)
        .args(["-t", &table, "-i", "zk", "queue", "-p", "host1", "join"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(stdout_json(&out)["ticket"]["counter"], 1);
}

#[test]
fn process_option_is_accepted_after_the_operation_too() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    let out = cli(&ep)
        .args(["-t", &table, "-i", "q", "queue", "join", "-p", "late"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["ticket"]["process_id"], "late");
}

#[test]
fn tag_value_may_contain_equals() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    let out = cli(&ep)
        .args([
            "-t",
            &table,
            "-i",
            "q",
            "queue",
            "-p",
            "a",
            "join",
            "--tag",
            "url=http://a=b",
        ])
        .output()
        .unwrap();
    assert_eq!(
        stdout_json(&out)["ticket"]["tags"],
        json!({"url": "http://a=b"})
    );
}

#[test]
fn explicit_region_and_table_are_echoed() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    let out = cli(&ep)
        .args(["-r", "us-east-1", "-t", &table, "-i", "c", "counter", "get"])
        .output()
        .unwrap();
    let v = stdout_json(&out);
    assert_eq!(v["region"], "us-east-1");
    assert_eq!(v["table"], json!(table));
}

#[test]
fn not_found_get_exits_1() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    cli(&ep)
        .args(["-t", &table, "-i", "q", "queue", "-p", "nobody", "get"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains(
            "ticket not found for process_id nobody",
        ));
}

#[test]
fn counter_command_on_queue_row_exits_1() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();
    cli(&ep)
        .args(["-t", &table, "-i", "shared", "queue", "-p", "a", "join"])
        .assert()
        .success();
    cli(&ep)
        .args(["-t", &table, "-i", "shared", "counter", "get"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "unrecognised structure type: expected COUNTER, found QUEUE",
        ));
}

#[test]
fn logging_goes_to_stderr_only() {
    let Some(ep) = endpoint() else { return };
    let table = unique_table();

    let quiet = cli(&ep)
        .args(["-t", &table, "-i", "c", "counter", "next"])
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(
        quiet.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );

    let loud = cli(&ep)
        .env("RUST_LOG", "debug")
        .args(["-t", &table, "-i", "c", "counter", "next"])
        .output()
        .unwrap();
    assert!(loud.status.success());
    assert!(!loud.stderr.is_empty());
    let v = stdout_json(&loud);
    assert_eq!(v["value"], 2);
}
