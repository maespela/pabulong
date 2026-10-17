use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=RNNOISE_LIB_DIR");
    println!("cargo:rerun-if-changed=build.rs");

    if env::var_os("CARGO_FEATURE_RNNOISE").is_none() {
        return;
    }

    // Default to the library vendored for the macOS app so `--features rnnoise`
    // works out of the box in this repository.
    let dir = env::var_os("RNNOISE_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("../../Sources/CRNNoise/lib")
        });
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=rnnoise");
}
