pub struct GeneratedPaths {
    pub swift_file: String,
    pub header_file: String,
    pub module_map: String,
    pub version: String,
}

pub fn build_bindings(language: &str) -> Result<GeneratedPaths, Box<dyn std::error::Error>> {
    println!("Self-building host cdylib for bindings generation...");
    
    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--package",
            "rust_auth_core",
            "--release",
        ])
        .status()?;

    if !status.success() {
        return Err("Failed to build host cdylib".into());
    }

    println!("Generating foreign bindings calling uniffi-bindgen binary...");
    let current_dir = std::env::current_dir()?;
    let lib_path = current_dir.join("target/release/librust_auth_core.dylib");
    
    let bindgen_status = std::process::Command::new("cargo")
        .args([
            "run",
            "--package",
            "rust_auth_core_builder",
            "--bin",
            "uniffi-bindgen",
            "generate",
            "--library",
            lib_path.to_str().unwrap(),
            "--language",
            language,
            "--out-dir",
            "target/generated-swift",
        ])
        .status()?;

    if !bindgen_status.success() {
        return Err("Failed to generate bindings".into());
    }

    let paths = GeneratedPaths {
        swift_file: "target/generated-swift/rust_auth_core.swift".to_string(),
        header_file: "target/generated-swift/rust_auth_coreFFI.h".to_string(),
        module_map: "target/generated-swift/rust_auth_coreFFI.modulemap".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
