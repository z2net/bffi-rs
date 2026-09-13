//! Materializes the loader JSON for the errors example module:
//! writes `.bffi/bffi.api.json` from the single `module_def::MODULE`
//! aggregation. The `@z2net/bffi` pipeline consumes that file.

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".bffi/bffi.api.json");
    if let Err(error) =
        bffi::build::loader_json::write_to_file(&bffi_example_errors::module_def::MODULE, &path)
    {
        eprintln!("emit-json: {error}");
        std::process::exit(1);
    }
    println!("written {}", path.display());
}
