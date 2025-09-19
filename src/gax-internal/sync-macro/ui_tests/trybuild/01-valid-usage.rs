#![allow(dead_code)]
use sync_keys_fields_macro::ensure_keys_and_fields_in_sync;

fn main() {
    #[ensure_keys_and_fields_in_sync(struct_name = "MyStruct")]
    mod test_struct_only {
        const KEY_USER_ID: &str = "user_id";
        struct MyStruct {
            user_id: String,
        }
    }

    #[ensure_keys_and_fields_in_sync(struct_name = "AnotherStruct", key_prefix = "ATTR")]
    mod test_struct_and_prefix {
        const ATTR_ITEM_ID: &str = "item_id";
        struct AnotherStruct {
            item_id: i64,
        }
    }
}