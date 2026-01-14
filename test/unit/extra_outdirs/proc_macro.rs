// Similar to
// https://github.com/napi-rs/napi-rs/blob/main/crates/macro/src/expand/typedef/type_def.rs#L11-L12
// this proc macro has a side-effect of writing extra metadata directories.
use proc_macro::TokenStream;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

#[proc_macro]
pub fn write_to_outdirs(_item: TokenStream) -> TokenStream {
    // Read the output directory paths from Bazel
    // Format: EXTRA_OUTDIRS_PATHS=dir1:path1,dir2:path2
    let outdirs_paths = env::var("EXTRA_OUTDIRS_PATHS")
        .expect("EXTRA_OUTDIRS_PATHS environment variable must be set");

    // Write to the output directories declared by Bazel
    for entry in outdirs_paths.split(',') {
        if let Some((_dir, path)) = entry.split_once(':') {
            let path_str = path.trim();
            // PathBuf will normalize the path correctly for the current platform
            // On Windows, ensure forward slashes are converted to backslashes
            let dir_path = if cfg!(windows) {
                // Convert forward slashes to backslashes for Windows
                let mut path_buf = PathBuf::from(path_str.replace('/', "\\"));
                // On Windows, ensure the path is absolute if it's not already
                if path_buf.is_relative() {
                    // Try to make it absolute by joining with current directory
                    if let Ok(current_dir) = std::env::current_dir() {
                        path_buf = current_dir.join(&path_buf);
                    }
                }
                path_buf
            } else {
                PathBuf::from(path_str)
            };

            // Create the directory if it doesn't exist
            // create_dir_all creates all parent directories as needed
            if let Err(e) = fs::create_dir_all(&dir_path) {
                panic!("Failed to create directory {}: {:?}", dir_path.display(), e);
            }
            // Write a marker file to ensure the directory is created
            // Use OpenOptions on Windows to ensure proper file creation
            let marker_file = dir_path.join("marker.txt");
            let result = if cfg!(windows) {
                // On Windows, use OpenOptions for more explicit control
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&marker_file)
                    .and_then(|mut file| file.write_all(b"created by proc-macro"))
            } else {
                fs::write(&marker_file, "created by proc-macro")
            };
            if let Err(e) = result {
                panic!(
                    "Failed to write marker file to {}: {:?}",
                    marker_file.display(),
                    e
                );
            }
        }
    }
    TokenStream::new()
}
