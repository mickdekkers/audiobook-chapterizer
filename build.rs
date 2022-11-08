use std::fs;
use std::path::PathBuf;

fn main() {
    let native_libs_dir = PathBuf::from("./native-libs");
    println!(
        "cargo:rustc-link-search={}",
        fs::canonicalize(native_libs_dir)
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap()
    );
    // Make the executable look for dynamic libraries in its own directory
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
}
