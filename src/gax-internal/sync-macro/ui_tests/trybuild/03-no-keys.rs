#![crate_type = "lib"]
use sync_keys_fields_macro::ensure_keys_and_fields_in_sync;

#[ensure_keys_and_fields_in_sync(struct_name = "MyStruct")]
mod test {
    struct MyStruct {
        foo: String,
    }
}
