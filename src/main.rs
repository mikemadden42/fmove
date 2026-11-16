//! fmove: sort the files in a directory into per-extension folders.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Folder name used for files that have no extension.
const NO_EXTENSION_DIR: &str = "Organized_Misc_Files";

/// Folder created inside Documents when no `--target` is given.
const DEFAULT_TARGET_SUBDIR: &str = "Sorted";

const USAGE: &str = "\
Usage: fmove [OPTIONS] <SOURCE_DIR>

Moves every regular file in SOURCE_DIR into <TARGET>/<extension>/.
Directories, hidden files and symlinks are left alone, and existing
files at the destination are never overwritten.

Options:
  -n, --dry-run        Show what would be moved without moving anything
  -y, --yes            Do not ask for confirmation
  -t, --target <DIR>   Target root directory (default: ~/Documents/Sorted)
  -h, --help           Print this help";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    source: PathBuf,
    target: Option<PathBuf>,
    dry_run: bool,
    yes: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Run(Options),
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut source = None;
    let mut target = None;
    let mut dry_run = false;
    let mut yes = false;

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => return Ok(Command::Help),
            Some("-n" | "--dry-run") => dry_run = true,
            Some("-y" | "--yes") => yes = true,
            Some("-t" | "--target") => {
                let dir = args.next().ok_or("--target requires a directory")?;
                target = Some(PathBuf::from(dir));
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option: {flag}"));
            }
            _ if source.is_some() => return Err("only one SOURCE_DIR may be given".to_string()),
            _ => source = Some(PathBuf::from(arg)),
        }
    }

    let source = source.ok_or("missing SOURCE_DIR")?;
    Ok(Command::Run(Options {
        source,
        target,
        dry_run,
        yes,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

impl From<fs::FileType> for EntryKind {
    fn from(ft: fs::FileType) -> Self {
        // DirEntry::file_type does not follow symlinks, so links are reported as such.
        if ft.is_symlink() {
            Self::Symlink
        } else if ft.is_dir() {
            Self::Dir
        } else if ft.is_file() {
            Self::File
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Move { dir: String },
    Skip(&'static str),
}

/// Decides what to do with a single directory entry. Pure, so it is easy to test.
fn decide(name: &OsStr, kind: EntryKind) -> Decision {
    match kind {
        EntryKind::File => {}
        EntryKind::Dir => return Decision::Skip("Directory"),
        EntryKind::Symlink => return Decision::Skip("Symlink"),
        EntryKind::Other => return Decision::Skip("Not a Regular File"),
    }
    if name.as_encoded_bytes().starts_with(b".") {
        return Decision::Skip("Hidden File");
    }
    // "name." has an empty extension; treat it like no extension at all.
    let dir = Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .filter(|ext| !ext.is_empty())
        .map_or_else(|| NO_EXTENSION_DIR.to_string(), str::to_lowercase);
    Decision::Move { dir }
}

struct PlannedMove {
    source: PathBuf,
    name: OsString,
    dir: String,
}

type Groups = BTreeMap<String, BTreeSet<String>>;

fn add(groups: &mut Groups, key: impl Into<String>, value: impl Into<String>) {
    groups.entry(key.into()).or_default().insert(value.into());
}

#[derive(Default)]
struct Plan {
    moves: Vec<PlannedMove>,
    skipped: Groups,
}

fn build_plan(source_dir: &Path) -> io::Result<Plan> {
    let mut plan = Plan::default();
    for entry in fs::read_dir(source_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                add(&mut plan.skipped, "Unreadable Entry", e.to_string());
                continue;
            }
        };
        let name = entry.file_name();
        let kind = match entry.file_type() {
            Ok(ft) => EntryKind::from(ft),
            Err(e) => {
                let label = format!("{}: {e}", name.to_string_lossy());
                add(&mut plan.skipped, "Unreadable Entry", label);
                continue;
            }
        };
        match decide(&name, kind) {
            Decision::Move { dir } => plan.moves.push(PlannedMove {
                source: entry.path(),
                name,
                dir,
            }),
            Decision::Skip(reason) => add(&mut plan.skipped, reason, name.to_string_lossy()),
        }
    }
    plan.moves.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(plan)
}

#[derive(Default)]
struct Report {
    moved: Groups,
    skipped: Groups,
    failed: Groups,
}

/// Carries out (or, with `dry_run`, simulates) the planned moves. Per-file
/// failures are recorded in the report instead of aborting the run.
fn execute(plan: Plan, target_root: &Path, dry_run: bool) -> Report {
    let mut report = Report {
        skipped: plan.skipped,
        ..Report::default()
    };
    let mut dir_status: HashMap<String, Result<(), String>> = HashMap::new();

    for planned in plan.moves {
        let label = planned.name.to_string_lossy().into_owned();
        let dest_dir = target_root.join(&planned.dir);
        let dest = dest_dir.join(&planned.name);

        if dry_run {
            if dest.exists() {
                add(&mut report.skipped, "Destination Exists", label);
            } else {
                add(&mut report.moved, planned.dir, label);
            }
            continue;
        }

        let dir_ok = dir_status
            .entry(planned.dir.clone())
            .or_insert_with(|| fs::create_dir_all(&dest_dir).map_err(|e| e.to_string()));
        if let Err(e) = dir_ok {
            let msg = format!("{label}: could not create {}: {e}", dest_dir.display());
            add(&mut report.failed, "Move Failed", msg);
            continue;
        }

        match move_no_clobber(&planned.source, &dest) {
            Ok(()) => add(&mut report.moved, planned.dir, label),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                add(&mut report.skipped, "Destination Exists", label);
            }
            Err(e) => add(&mut report.failed, "Move Failed", format!("{label}: {e}")),
        }
    }
    report
}

/// Moves `src` to `dst`, failing with `AlreadyExists` instead of overwriting.
///
/// A hard link is created first because it atomically refuses to replace an
/// existing file. If linking is not possible (different filesystem, or one that
/// does not support links) the file is copied into a newly created file instead.
fn move_no_clobber(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::hard_link(src, dst) {
        Ok(()) => fs::remove_file(src).inspect_err(|_| {
            let _ = fs::remove_file(dst);
        }),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => copy_then_remove(src, dst),
    }
}

fn copy_then_remove(src: &Path, dst: &Path) -> io::Result<()> {
    let mut input = File::open(src)?;
    let meta = input.metadata()?;
    // create_new makes the existence check and the creation a single step.
    let mut output = OpenOptions::new().write(true).create_new(true).open(dst)?;

    let copied = copy_contents(&mut input, &mut output, &meta);
    // Close both handles before removing anything; Windows requires it.
    drop(input);
    drop(output);

    let result = copied.and_then(|()| fs::remove_file(src));
    if result.is_err() {
        let _ = fs::remove_file(dst);
    }
    result
}

fn copy_contents(input: &mut File, output: &mut File, meta: &Metadata) -> io::Result<()> {
    io::copy(input, output)?;
    if let Ok(modified) = meta.modified() {
        output.set_modified(modified)?;
    }
    output.sync_all()?;
    output.set_permissions(meta.permissions())
}

fn print_groups(title: &str, groups: &Groups, heading: impl Fn(&str) -> String) {
    if groups.is_empty() {
        return;
    }
    println!("\n--- {title} ---");
    for (key, names) in groups {
        println!("\n- {}:", heading(key));
        for name in names {
            println!("  - {name}");
        }
    }
}

fn print_report(report: &Report, target_root: &Path, dry_run: bool) {
    let root = target_root.display();
    let (title, verb) = if dry_run {
        ("Files That Would Be Moved", "would move to")
    } else {
        ("Files Moved", "moved to")
    };
    print_groups(title, &report.moved, |ext| {
        format!("Files {verb} {root}/{ext}")
    });
    print_groups("Files Not Moved (Skipped)", &report.skipped, str::to_string);
    print_groups("Errors", &report.failed, str::to_string);

    let count = |g: &Groups| g.values().map(BTreeSet::len).sum::<usize>();
    println!(
        "\n{} {}, {} skipped, {} failed.",
        count(&report.moved),
        if dry_run { "would be moved" } else { "moved" },
        count(&report.skipped),
        count(&report.failed),
    );
}

fn confirm(prompt: &str) -> io::Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn run(options: Options) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let source = fs::canonicalize(&options.source)
        .map_err(|e| format!("cannot open {}: {e}", options.source.display()))?;
    let target_root = match options.target {
        Some(target) => target,
        None => dirs::document_dir()
            .ok_or("could not determine the Documents directory; pass --target")?
            .join(DEFAULT_TARGET_SUBDIR),
    };

    println!("Source directory: {}", source.display());
    println!("Target root:      {}", target_root.display());

    let plan = build_plan(&source)?;
    if plan.moves.is_empty() {
        println!("\nNo files to move.");
    } else if !options.dry_run && !options.yes {
        if !io::stdin().is_terminal() {
            return Err("refusing to move files without confirmation; pass --yes".into());
        }
        let prompt = format!(
            "\nMove {} file(s) into {}?",
            plan.moves.len(),
            target_root.display()
        );
        if !confirm(&prompt)? {
            println!("Aborted; nothing was moved.");
            return Ok(ExitCode::SUCCESS);
        }
    }

    let report = execute(plan, &target_root, options.dry_run);
    print_report(&report, &target_root, options.dry_run);
    Ok(if report.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args_os().skip(1)) {
        Ok(Command::Run(options)) => options,
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Command, String> {
        parse_args(list.iter().map(OsString::from))
    }

    fn moved_to(dir: &str) -> Decision {
        Decision::Move {
            dir: dir.to_string(),
        }
    }

    #[test]
    fn parse_requires_source() {
        assert!(args(&[]).is_err());
        assert!(args(&["-n"]).is_err());
    }

    #[test]
    fn parse_flags() {
        assert_eq!(
            args(&["-n", "--yes", "--target", "/out", "src"]),
            Ok(Command::Run(Options {
                source: PathBuf::from("src"),
                target: Some(PathBuf::from("/out")),
                dry_run: true,
                yes: true,
            }))
        );
        assert_eq!(args(&["src", "-h"]), Ok(Command::Help));
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(args(&["--bogus", "src"]).is_err());
        assert!(args(&["a", "b"]).is_err());
        assert!(args(&["src", "--target"]).is_err());
    }

    #[test]
    fn decide_by_extension() {
        let file = EntryKind::File;
        assert_eq!(decide(OsStr::new("a.PDF"), file), moved_to("pdf"));
        assert_eq!(decide(OsStr::new("a.tar.gz"), file), moved_to("gz"));
        assert_eq!(
            decide(OsStr::new("Makefile"), file),
            moved_to(NO_EXTENSION_DIR)
        );
        assert_eq!(
            decide(OsStr::new("trailing."), file),
            moved_to(NO_EXTENSION_DIR)
        );
    }

    #[test]
    fn decide_skips_non_files() {
        assert_eq!(
            decide(OsStr::new(".bashrc"), EntryKind::File),
            Decision::Skip("Hidden File")
        );
        assert_eq!(
            decide(OsStr::new("x.txt"), EntryKind::Dir),
            Decision::Skip("Directory")
        );
        assert_eq!(
            decide(OsStr::new("x.txt"), EntryKind::Symlink),
            Decision::Skip("Symlink")
        );
    }

    #[test]
    fn move_refuses_to_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("a"), tmp.path().join("b"));
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        let err = move_no_clobber(&src, &dst).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&src).unwrap(), "new");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn copy_fallback_moves_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("a"), tmp.path().join("b"));
        fs::write(&src, "data").unwrap();

        copy_then_remove(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "data");

        fs::write(&src, "again").unwrap();
        let err = copy_then_remove(&src, &dst).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "data");
    }

    fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(source.path().join("doc.PDF"), "pdf").unwrap();
        fs::write(source.path().join("notes"), "txt").unwrap();
        fs::write(source.path().join(".hidden"), "").unwrap();
        fs::create_dir(source.path().join("subdir")).unwrap();
        fs::create_dir(target.path().join("pdf")).unwrap();
        (source, target)
    }

    #[test]
    fn organizes_directory() {
        let (source, target) = setup();
        fs::write(source.path().join("clash.txt"), "new").unwrap();
        fs::create_dir(target.path().join("txt")).unwrap();
        fs::write(target.path().join("txt/clash.txt"), "old").unwrap();

        let plan = build_plan(source.path()).unwrap();
        let report = execute(plan, target.path(), false);

        assert!(report.failed.is_empty());
        assert_eq!(
            fs::read_to_string(target.path().join("pdf/doc.PDF")).unwrap(),
            "pdf"
        );
        assert!(target.path().join(NO_EXTENSION_DIR).join("notes").exists());
        assert!(!source.path().join("doc.PDF").exists());
        assert!(source.path().join(".hidden").exists());
        assert!(source.path().join("subdir").is_dir());
        assert_eq!(
            fs::read_to_string(source.path().join("clash.txt")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(target.path().join("txt/clash.txt")).unwrap(),
            "old"
        );
        assert!(report.skipped["Destination Exists"].contains("clash.txt"));
        assert!(report.skipped["Hidden File"].contains(".hidden"));
        assert!(report.skipped["Directory"].contains("subdir"));
    }

    #[test]
    fn dry_run_moves_nothing() {
        let (source, target) = setup();
        let plan = build_plan(source.path()).unwrap();
        let report = execute(plan, target.path(), true);

        assert!(report.moved["pdf"].contains("doc.PDF"));
        assert!(source.path().join("doc.PDF").exists());
        assert!(!target.path().join(NO_EXTENSION_DIR).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped() {
        let (source, target) = setup();
        std::os::unix::fs::symlink("doc.PDF", source.path().join("link.pdf")).unwrap();
        std::os::unix::fs::symlink("missing", source.path().join("broken")).unwrap();

        let plan = build_plan(source.path()).unwrap();
        let report = execute(plan, target.path(), false);

        let links = &report.skipped["Symlink"];
        assert!(links.contains("link.pdf") && links.contains("broken"));
        assert!(fs::symlink_metadata(source.path().join("link.pdf")).is_ok());
    }
}
