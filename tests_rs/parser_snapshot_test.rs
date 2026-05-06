//! Snapshot tests pinning the Python-binding-shape AST dict.
//!
//! Two suites:
//!   1. `golden_asts` — compares `parse_to_compat_json` against the
//!      pre-existing `tests/fixtures/ast/golden_asts.json` corpus.
//!   2. `synthetic` — compares against `tests_rs/snapshots/synthetic_expected.json`,
//!      which is regenerated when `UPDATE_SNAPSHOTS=1` is set.
//!
//! When the parser later swaps internals, this harness proves the FFI dict
//! shape is byte-for-byte identical.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};

use _rust::compat::parse_to_compat_json;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &PathBuf) -> Value {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("failed to parse JSON {}: {}", path.display(), e))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| "<unserializable>".into())
}

/// Compare two values; on mismatch, panic with both pretty-printed JSONs.
fn assert_value_eq(expr: &str, actual: &Value, expected: &Value) {
    if actual != expected {
        panic!(
            "AST mismatch for expression: {expr:?}\n--- expected ---\n{}\n--- actual ---\n{}\n",
            pretty(expected),
            pretty(actual),
        );
    }
}

#[test]
fn golden_asts_match_parser_output() {
    let path = manifest_dir().join("tests/fixtures/ast/golden_asts.json");
    let golden = read_json(&path);
    let map = match &golden {
        Value::Object(m) => m,
        _ => panic!("golden_asts.json must be a top-level object"),
    };

    let mut failures: Vec<(String, String)> = Vec::new();
    let mut compared = 0usize;

    for (expr, expected) in map {
        match parse_to_compat_json(expr) {
            Ok(actual) => {
                if &actual != expected {
                    failures.push((
                        expr.clone(),
                        format!(
                            "AST mismatch.\n--- expected ---\n{}\n--- actual ---\n{}\n",
                            pretty(expected),
                            pretty(&actual),
                        ),
                    ));
                } else {
                    compared += 1;
                }
            }
            Err(e) => {
                failures.push((expr.clone(), format!("parse error: {e}")));
            }
        }
    }

    if !failures.is_empty() {
        let mut msg = format!(
            "{} of {} golden expressions failed to match (compared OK: {}):\n",
            failures.len(),
            map.len(),
            compared,
        );
        for (expr, why) in &failures {
            msg.push_str(&format!("\n  EXPR: {expr:?}\n  {why}\n"));
        }
        panic!("{msg}");
    }
}

#[test]
fn synthetic_corpus_matches_baseline() {
    let corpus_path = manifest_dir().join("tests_rs/snapshots/synthetic_corpus.json");
    let expected_path = manifest_dir().join("tests_rs/snapshots/synthetic_expected.json");
    let corpus = read_json(&corpus_path);
    let exprs: Vec<String> = match corpus {
        Value::Array(a) => a
            .into_iter()
            .map(|v| match v {
                Value::String(s) => s,
                other => panic!("synthetic_corpus.json entries must be strings, got {other}"),
            })
            .collect(),
        _ => panic!("synthetic_corpus.json must be a JSON array of strings"),
    };

    let update = std::env::var("UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if update {
        // Use BTreeMap for deterministic output ordering.
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        for expr in &exprs {
            let v = parse_to_compat_json(expr).unwrap_or_else(|e| {
                panic!("UPDATE_SNAPSHOTS: parse failed for {expr:?}: {e}")
            });
            out.insert(expr.clone(), v);
        }
        let mut map = Map::new();
        for (k, v) in out {
            map.insert(k, v);
        }
        let serialized = serde_json::to_string_pretty(&Value::Object(map)).unwrap();
        fs::write(&expected_path, format!("{serialized}\n")).unwrap_or_else(|e| {
            panic!("failed to write {}: {}", expected_path.display(), e)
        });
        eprintln!(
            "UPDATE_SNAPSHOTS=1: wrote {} entries to {}",
            exprs.len(),
            expected_path.display()
        );
        return;
    }

    let expected = read_json(&expected_path);
    let expected_map = match &expected {
        Value::Object(m) => m,
        _ => panic!("synthetic_expected.json must be a top-level object"),
    };

    for expr in &exprs {
        let actual = parse_to_compat_json(expr)
            .unwrap_or_else(|e| panic!("parse failed for {expr:?}: {e}"));
        let exp = expected_map
            .get(expr)
            .unwrap_or_else(|| panic!("missing baseline for {expr:?}; rerun with UPDATE_SNAPSHOTS=1"));
        assert_value_eq(expr, &actual, exp);
    }

    // Detect drift: an entry in expected but not the corpus suggests stale snapshot.
    for k in expected_map.keys() {
        if !exprs.iter().any(|e| e == k) {
            panic!(
                "stale entry in synthetic_expected.json (not in corpus): {k:?}; \
                 rerun with UPDATE_SNAPSHOTS=1 to regenerate"
            );
        }
    }
}
