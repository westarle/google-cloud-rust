pub fn map_swift_to_rust_target(swift_target: &str) -> &str {
    match swift_target {
        "x86_64-swift-linux-musl" => "x86_64-unknown-linux-musl",
        "aarch64-swift-linux-musl" => "aarch64-unknown-linux-musl",
        "x86_64-unknown-windows-msvc" => "x86_64-pc-windows-msvc",
        "aarch64-unknown-windows-msvc" => "aarch64-pc-windows-msvc",
        "wasm32-unknown-wasi" => "wasm32-wasip1",
        "x86_64-apple-macosx" => "x86_64-apple-darwin",
        "arm64-apple-macosx" => "aarch64-apple-darwin",
        _ => swift_target,
    }
}
