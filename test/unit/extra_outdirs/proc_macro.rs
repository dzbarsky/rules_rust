// Similar to
// https://github.com/napi-rs/napi-rs/blob/main/crates/macro/src/expand/typedef/type_def.rs#L11-L12
// this proc macro has a side-effect of writing extra metadata directories.
use proc_macro::TokenStream;
use std::env;
use std::fs;
use std::path::PathBuf;

#[proc_macro]
pub fn write_to_outdirs(_item: TokenStream) -> TokenStream {
    // Read the list of directories to write to from an environment variable
    let outdirs = env::var("EXTRA_OUTDIRS")
        .expect("EXTRA_OUTDIRS environment variable must be set");
    
    // Get the manifest directory (package directory) as the base path
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| ".".to_string());
    
    for dir in outdirs.split(',') {
        let dir = dir.trim();
        if !dir.is_empty() {
            // Construct the full path: manifest_dir + directory name
            let dir_path = PathBuf::from(&manifest_dir).join(dir);
            // Create the directory if it doesn't exist
            if let Err(e) = fs::create_dir_all(&dir_path) {
                panic!("Failed to create directory {}: {:?}", dir_path.display(), e);
            }
            // Write a marker file to ensure the directory is created
            let marker_file = dir_path.join("marker.txt");
            if let Err(e) = fs::write(&marker_file, "created by proc-macro") {
                panic!("Failed to write marker file to {}: {:?}", marker_file.display(), e);
            }
        }
    }
    TokenStream::new()
}
