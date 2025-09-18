#[test]
fn trybuild() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/success/base.rs");
    t.pass("tests/trybuild/success/options.rs");
    t.compile_fail("tests/trybuild/fail/missing_key.rs");
    t.compile_fail("tests/trybuild/fail/unsupported_type.rs");
}