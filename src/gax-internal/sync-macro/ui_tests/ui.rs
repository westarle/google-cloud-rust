#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.pass("ui_tests/trybuild/01-valid-usage.rs");
    t.compile_fail("ui_tests/trybuild/02-struct-not-found.rs");
    t.compile_fail("ui_tests/trybuild/03-no-keys.rs");
    t.compile_fail("ui_tests/trybuild/04-key-wrong-type.rs");
    t.compile_fail("ui_tests/trybuild/05-key-without-field.rs");
    t.compile_fail("ui_tests/trybuild/06-field-without-key.rs");
}
