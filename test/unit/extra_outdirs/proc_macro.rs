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
    
    // Read the output directory paths from Bazel
    // Format: EXTRA_OUTDIRS_PATHS=dir1:path1,dir2:path2
    let outdirs_paths = env::var("EXTRA_OUTDIRS_PATHS")
        .expect("EXTRA_OUTDIRS_PATHS environment variable must be set");
    
    // Create a map of directory name to output path
    let mut path_map = std::collections::HashMap::new();
    for entry in outdirs_paths.split(',') {
        if let Some((dir, path)) = entry.split_once(':') {
            path_map.insert(dir.trim(), path.trim());
        }
    }
    
    // Write to the output directories declared by Bazel
    for dir in outdirs.split(',') {
        let dir = dir.trim();
        if !dir.is_empty() {
            // Get the output path for this directory
            let dir_path = if let Some(path) = path_map.get(dir) {
                PathBuf::from(path)
            } else {
                // Fallback to directory name if path not found
                PathBuf::from(dir)
            };
            
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
