use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

// Placeholder for files with no extension to use as a directory name.
const NO_EXTENSION_DIR: &str = "Organized_Misc_Files";

/// Attempts to find and return the user's Documents directory.
/// Requires the 'dirs' crate for cross-platform compatibility.
fn get_documents_dir() -> Result<PathBuf, String> {
    dirs::document_dir().ok_or_else(|| "Could not determine the Documents directory.".to_string())
}

/// Reads the current directory, moves files into extension-based folders
/// inside the Documents directory, and handles file conflicts by skipping.
fn run_file_organizer() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Determine target directories
    let current_dir = env::current_dir()?;
    let documents_root = get_documents_dir()?;

    // FIX: Use named arguments in format! to avoid warnings
    println!("Source Directory: {}", current_dir.display());
    println!("Target Root Directory: {}\n", documents_root.display());

    // A map to track files that were successfully moved
    let mut files_moved = BTreeMap::new();
    // A map to track files that were intentionally skipped
    let mut files_skipped = BTreeMap::new();

    // 2. Iterate through the current directory
    for entry in fs::read_dir(&current_dir)? {
        let entry = entry?;
        let source_path = entry.path();

        // Check if the entry is a directory and skip it
        if source_path.is_dir() {
            let filename = source_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            files_skipped
                .entry("Directory".to_string())
                .or_insert_with(Vec::new)
                .push(filename); // FIX: Use filename directly
            continue;
        }

        // Only process files
        if source_path.is_file() {
            // FIX is correct here: changed s.to_str to s.to_str()
            if let Some(filename) = source_path.file_name().and_then(|s| s.to_str()) {
                // Check if the file is hidden and skip it
                if filename.starts_with('.') {
                    files_skipped
                        .entry("Hidden File".to_string())
                        .or_insert_with(Vec::new)
                        .push(filename.to_string()); // FIX: Use filename directly
                    continue;
                }

                // Determine the target extension directory name
                let extension_dir_name = match source_path.extension().and_then(|ext| ext.to_str())
                {
                    Some(ext) => ext.to_lowercase(),
                    None => NO_EXTENSION_DIR.to_string(),
                };

                // 3. Construct the destination directory and ensure it exists
                let dest_dir = documents_root.join(&extension_dir_name);
                fs::create_dir_all(&dest_dir)?;

                // 4. Construct the final destination path
                let final_dest_path = dest_dir.join(filename);

                // 5. NEW LOGIC: Check for existence and skip if found
                if final_dest_path.exists() {
                    files_skipped
                        .entry("Destination Exists".to_string())
                        .or_insert_with(Vec::new)
                        .push(filename.to_string()); // FIX: Use filename directly
                    continue; // Skip the move operation
                }

                // 6. Move the file (only if it didn't exist)
                // fs::rename works cross-platform for move/rename operations.
                fs::rename(&source_path, &final_dest_path)?;

                // 7. Record the move
                let final_name = final_dest_path
                    .file_name()
                    .unwrap_or(source_path.file_name().unwrap())
                    .to_string_lossy();
                files_moved
                    .entry(extension_dir_name.clone())
                    .or_insert_with(Vec::new)
                    // FIX: Use named arguments in format! to avoid warnings
                    .push(format!("{filename} -> {final_name}"));
            } else {
                // Handle paths that are not valid UTF-8
                let lossy_path = source_path.to_string_lossy().to_string();
                files_skipped
                    .entry("Invalid Filename".to_string())
                    .or_insert_with(Vec::new)
                    .push(lossy_path); // FIX: Use lossy_path directly
            }
        }
    }

    // 8. Report the results
    println!("--- Files Successfully Moved ---");
    let final_report_path = documents_root.display();
    for (ext, filenames) in files_moved {
        // FIX: Use named arguments in format! to avoid warnings
        println!("\n- Files moved to {final_report_path}/{ext}:");
        // Sort filenames for a clean report output
        let mut sorted_filenames = filenames;
        sorted_filenames.sort_unstable();
        for name in sorted_filenames {
            // FIX: Use named arguments in format! to avoid warnings
            println!("  - {name}");
        }
    }

    // 9. Report the skipped files
    if !files_skipped.is_empty() {
        println!("\n--- Files Not Moved (Skipped) ---");
        for (reason, filenames) in files_skipped {
            // FIX: Use named arguments in format! to avoid warnings
            println!("\n- {reason}:");
            let mut sorted_filenames = filenames;
            sorted_filenames.sort_unstable();
            for name in sorted_filenames {
                // FIX: Use named arguments in format! to avoid warnings
                println!("  - {name}");
            }
        }
    }

    // FIX: Use named arguments in format! to avoid warnings
    println!(
        "\nOrganization complete! Files are now located in subdirectories within: {final_report_path}"
    );

    Ok(())
}

fn main() {
    match run_file_organizer() {
        Ok(()) => {}
        Err(e) => {
            // FIX: Use named arguments in eprintln! to avoid warnings
            eprintln!("Error organizing files: {e}");
            std::process::exit(1);
        }
    }
}
