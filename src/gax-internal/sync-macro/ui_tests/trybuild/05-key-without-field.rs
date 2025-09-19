#![crate_type = "lib"]
use sync_keys_fields_macro::ensure_keys_and_fields_in_sync;

#[ensure_keys_and_fields_in_sync(struct_name = "MyStruct")]
mod test {
    const KEY_FOO: &str = "foo";
    const KEY_BAR: &str = "bar"; // No bar field
    struct MyStruct {
        foo: String,
    }
}
