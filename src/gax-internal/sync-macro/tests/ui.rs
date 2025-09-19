#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/01-valid-usage.rs");
}
