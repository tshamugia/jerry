//! The naming rules against the table the UI's own check is tested with
//! (`src/features/rename/name.test.ts`): both must agree on every case.

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use jerry_core::name::check_name;
use serde_json::Value;

#[test]
fn rules_match_the_shared_table() {
    let table: Value =
        serde_json::from_str(include_str!("name_cases.json")).expect("the table is JSON");
    let cases = table["cases"].as_array().expect("cases");
    assert!(cases.len() > 40, "the table has its cases");
    for case in cases {
        let part = case["name"].as_str().expect("a name");
        let repeat = case["repeat"].as_u64().unwrap_or(1) as usize;
        let name = part.repeat(repeat);
        let expected = case["problem"].as_str();
        let found = check_name(&name).err().map(|problem| problem.kind());
        assert_eq!(found, expected, "{part:?} × {repeat}");
    }
}
