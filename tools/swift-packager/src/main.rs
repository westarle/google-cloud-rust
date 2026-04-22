mod common;
mod xcframework;
mod artifactbundle;

use clap::Parser;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Target triples to build for
    #[arg(long, default_value = "x86_64-unknown-linux-gnu", value_delimiter = ',')]
    target: Vec<String>,
}



fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    println!("Building Rust crate for Swift bindings for targets {:?}...", cli.target);

    let paths = rust_auth_core_builder::build_bindings("swift")?;
    let version = paths.version.clone();
    println!("Using version {} received from library API", version);

    // Create directory for generated Swift files in the package
    let package_src_dir = Path::new("target/GeneratedPackage/Sources/GoogleCloudAuthInternal");
    fs::create_dir_all(package_src_dir)?;

    // Copy generated Swift file
    fs::copy(&paths.swift_file, package_src_dir.join("rust_auth_core.swift"))?;

    // Call modules to bind results together
    xcframework::generate_xcframework(&cli.target, &paths)?;
    artifactbundle::generate_artifactbundle(&cli.target, &paths, &version)?;

    Ok(())
}
