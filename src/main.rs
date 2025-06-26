// A safer version of cargo publish
//
// Copyright (C) 2025 Georg Semmler
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see
// <https://www.gnu.org/licenses/>.

use colored::Colorize;
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_GENERATED_FILES: &[&str] = &[".cargo_vcs_info.json", "Cargo.toml"];
const REMAP_FILES: [(&str, &str); 1] = [("Cargo.toml.orig", "Cargo.toml")];

#[derive(serde_derive::Deserialize, Debug)]
struct IncludeExcludeFromManifest {
    package: PackageIncludeExelude,
}

#[derive(serde_derive::Deserialize, Debug)]
struct PackageIncludeExelude {
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

fn manifest_path() -> Option<String> {
    let mut args = std::env::args().skip_while(|c| !c.starts_with("--manifest-path"));
    match args.next() {
        Some(n) if n.starts_with("--manifest-path=") => {
            n.strip_prefix("--manifest-path=").map(|t| t.to_owned())
        }
        Some(_) => args.next(),
        None => None,
    }
}

fn package_flag() -> Option<String> {
    let mut args =
        std::env::args().skip_while(|c| !(c.starts_with("--package") || c.starts_with("-p")));
    match args.next() {
        Some(n) if n.starts_with("--package=") => {
            n.strip_prefix("--package=").map(|t| t.to_owned())
        }
        Some(_) => args.next(),
        None => None,
    }
}

fn verify_content_matches(
    package_root: &cargo_metadata::camino::Utf8Path,
    package_version: &cargo_metadata::semver::Version,
    package_name: &str,
) -> bool {
    let body = ureq::get(format!(
        "https://crates.io/api/v1/crates/{package_name}/{package_version}/download"
    ))
    .header("User-Agent", format!("cargo-safe-publish/{APP_VERSION}"))
    .call()
    .expect("Failed to fetch package")
    .body_mut()
    .read_to_vec()
    .expect("Failed to fetch package");
    let remapped_files = HashMap::from(REMAP_FILES);

    let zipped_archive = GzDecoder::new(std::io::Cursor::new(body));
    let mut archive = tar::Archive::new(zipped_archive);
    let mut everything_matched = true;
    for entry in archive
        .entries()
        .expect("Could not open uploaded `.crate` archive")
    {
        let mut entry = entry.expect("Failed to get file entry from tar archive");

        let path = entry.path().unwrap().into_owned();
        let mut package_local_path = path
            .strip_prefix(format!("{package_name}-{package_version}"))
            .unwrap()
            .to_path_buf();

        // we want to make sure that we compare `Cargo.toml.orig` to the local `Cargo.toml` as otherwise
        // they don't match
        if let Some(remap_file) = remapped_files.get(path.file_name().unwrap().to_str().unwrap()) {
            package_local_path = package_local_path.parent().unwrap().join(*remap_file);
        }

        let local_path = package_root.join(package_local_path.display().to_string());
        if !CARGO_GENERATED_FILES.contains(&path.file_name().unwrap().to_str().unwrap()) {
            if local_path.exists() {
                let mut uploaded_content = String::new();
                entry
                    .read_to_string(&mut uploaded_content)
                    .expect("Failed to read file from tar archive");
                let local_content =
                    std::fs::read_to_string(local_path).expect("Could not read local file");
                if local_content != uploaded_content {
                    let diff = similar_asserts::SimpleDiff::from_str(
                        &local_content,
                        &uploaded_content,
                        "Local version",
                        "Uploaded version",
                    );
                    eprintln!(
                        "{}: found differences in `{}`:",
                        "error".red().bold(),
                        package_local_path.display().to_string().bold()
                    );
                    eprintln!("{diff}");
                    everything_matched = false;
                }
            } else {
                eprintln!(
                    "{}: the file `{path}` does not exist in `{package_root}`",
                    "error".red().bold(),
                    path = package_local_path.display().to_string().bold(),
                );
                everything_matched = false;
            }
        }
    }
    everything_matched
}

fn run_publish() {
    let mut publish_command = Command::new("cargo");

    publish_command
        .arg("publish")
        .arg("--no-verify")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    // append all the other flags
    for arg in std::env::args().skip(1).filter(|c| c != "--no-verify") {
        publish_command.arg(arg);
    }

    println!("Run cargo publish with the following command: `{publish_command:?}`");
    let publish_status = publish_command.status();
    match publish_status {
        Err(e) => {
            eprintln!("{}: publish run failed: {e}", "error".red().bold());
            std::process::exit(1);
        }
        Ok(s) if !s.success() => {
            eprintln!(
                "{}: publish run returned a non-zero exist code, check the output above for details",
                "error".red().bold()
            );
            std::process::exit(s.code().unwrap_or(1));
        }
        Ok(_) => {}
    }
}

fn run_verification_build(
    target_directory: &Path,
    package_name: &str,
    package_version: &cargo_metadata::semver::Version,
) {
    let mut dry_run_command = Command::new("cargo");

    dry_run_command
        .arg("publish")
        .arg("--dry-run")
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit());

    // append all the other flags
    for arg in std::env::args().skip(1).filter(|c| c != "--dry-run") {
        dry_run_command.arg(arg);
    }
    println!("Run verification build with the following command: `{dry_run_command:?}`");
    let dry_run_status = dry_run_command.status();
    match dry_run_status {
        Err(e) => {
            eprintln!("{}: dry run failed: {e}", "error".red().bold());
            std::process::exit(1);
        }
        Ok(s) if !s.success() => {
            eprintln!(
                "{}: dry run returned a non-zero exist code, check the output above for details",
                "error".red().bold()
            );
            std::process::exit(s.code().unwrap_or(1));
        }
        Ok(_) => {}
    }

    // cargo should remove these files on it's own on the new call to `cargo publish` with the same version
    // but we better make sure that they are gone instead of relying on that behavior
    let unpacked_target_package = target_directory
        .join("package")
        .join(format!("{package_name}-{package_version}"));
    let target_package = target_directory
        .join("package")
        .join(format!("{package_name}-{package_version}.crate"));

    std::fs::remove_dir_all(unpacked_target_package).expect(
        "Failed to remove unpacked package from the target directory during the verification build",
    );
    std::fs::remove_file(target_package).expect(
        "Failed to remove the packed crate from the target directory during the verification build",
    );
}

fn get_git_root(package_root: &Path) -> Option<&Path> {
    let mut check_dir = Some(package_root);
    loop {
        let local_check_dir = check_dir?;
        if local_check_dir.join(".git").exists() {
            return Some(local_check_dir);
        } else {
            check_dir = local_check_dir.parent();
        }
    }
}

fn check_git_is_dirty(package_root: &cargo_metadata::camino::Utf8Path) {
    if let Some(git_root) = get_git_root(package_root.as_std_path()) {
        let manifest = std::fs::read_to_string(package_root.join("Cargo.toml"))
            .expect("Failed to read `Cargo.toml`");
        let manifest: IncludeExcludeFromManifest =
            toml::de::from_str(&manifest).expect("Failed to deserialize `Cargo.toml`");
        if manifest.package.include.is_some() && manifest.package.exclude.is_some() {
            eprintln!(
                "{}: both `package.include` and `package.exclude` are set. Cargo will ignore `package.exclude` in this case",
                "warning".yellow()
            );
        }

        let include = manifest.package.include.as_deref().map(|p| {
            p.iter()
                .fold(
                    ignore::gitignore::GitignoreBuilder::new(package_root),
                    |mut builder, i| {
                        builder.add_line(None, i).unwrap();
                        builder
                    },
                )
                .build()
                .unwrap()
        });
        let exclude = manifest.package.exclude.as_deref().map(|p| {
            p.iter()
                .fold(
                    ignore::gitignore::GitignoreBuilder::new(package_root),
                    |mut builder, i| {
                        builder.add_line(None, i).unwrap();
                        builder
                    },
                )
                .build()
                .unwrap()
        });

        let (patterns, sub_dir) = if package_root == git_root {
            (
                Box::new(std::iter::empty()) as Box<dyn Iterator<Item = _>>,
                None,
            )
        } else {
            let package_path_in_git = package_root
                .as_std_path()
                .strip_prefix(git_root)
                .expect("The package_root path is a child path or equivalent to the git root path");
            let only_in_subdir = gix::diff::object::bstr::BString::new(
                package_path_in_git.as_os_str().as_encoded_bytes().to_vec(),
            );
            (
                Box::new(std::iter::once(only_in_subdir.clone())) as Box<dyn Iterator<Item = _>>,
                Some(only_in_subdir),
            )
        };

        let repo = gix::open(git_root).expect("Could not open git repo");
        let status = repo
            .status(gix::progress::Discard)
            .expect("Failed to get repo state")
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(patterns)
            .expect("Failed to get repo state")
            .filter_map(|i| {
                let item = match i {
                    Ok(i) => i,
                    Err(e) => return Some(Err(e)),
                };
                let mut path = item.location();

                if let Some(sub_dir) = &sub_dir {
                    path = gix::diff::object::bstr::BStr::new(
                        path.strip_prefix(sub_dir.as_slice()).unwrap(),
                    );
                    path = gix::diff::object::bstr::BStr::new(
                        path.strip_prefix(&[std::path::MAIN_SEPARATOR as u8])
                            .unwrap(),
                    );
                }
                // we don't want to filter out submodule modifications, so just don't check if they are included or not
                if !matches!(
                    &item,
                    gix::status::Item::IndexWorktree(
                        gix::status::index_worktree::Item::Modification {
                            status:
                            gix::status::plumbing::index_as_worktree::EntryStatus::Change(gix::status::plumbing::index_as_worktree::Change::SubmoduleModification{..}),
                            ..
                        })
                ) {
                    let path_to_check = <[u8] as gix::diff::object::bstr::ByteSlice>::to_path(path).expect("Valid OsStr");
                    let is_dir = false;
                    if let Some(includes) = &include {
                        if !includes.matched_path_or_any_parents(path_to_check, is_dir).is_ignore() {
                            return None;
                        }
                    } else if let Some(excludes) = &exclude {
                        if excludes.matched_path_or_any_parents(path_to_check, is_dir).is_ignore() {
                            return None;
                        }
                    }
                }
                let path = path.to_owned();
                Some(Ok((item, path)))
            })
            .collect::<Result<Vec<_>, _>>()
            .expect("Failed to get repo state");

        if !status.is_empty() {
            eprintln!();
            eprintln!(
                "{}: {} files in the working directory contain changes \
                     that were not yet committed into git:",
                "error".red().bold(),
                status.len()
            );
            eprintln!();
            for (item, path) in status {
                let modification_kind = match &item {
                    gix::status::Item::IndexWorktree(
                        gix::status::index_worktree::Item::DirectoryContents { entry, .. },
                    ) => format!(" ({:?})", entry.status),
                    gix::status::Item::IndexWorktree(
                        gix::status::index_worktree::Item::Modification {
                            status:
                                gix::status::plumbing::index_as_worktree::EntryStatus::Change(gix::status::plumbing::index_as_worktree::Change::Modification { .. }),
                            ..
                        },
                    ) => " (Modified)".to_owned(),
                    gix::status::Item::IndexWorktree(
                        gix::status::index_worktree::Item::Modification {
                            status:
                                gix::status::plumbing::index_as_worktree::EntryStatus::Change(gix::status::plumbing::index_as_worktree::Change::SubmoduleModification { .. }),
                            ..
                        },
                    ) => " (Submodule Modified)".to_owned(),
                    _ => "".to_owned(),
                };
                eprintln!("{path}{modification_kind}", path = path.to_string().bold());
            }

            std::process::exit(1);
        }
    }
}

fn main() {
    let is_dry_run = std::env::args().any(|c| c == "--dry-run");
    let is_no_verify = std::env::args().any(|c| c == "--no-verify");
    let is_help = std::env::args().any(|c| c == "--help" || c == "-h");
    let is_allow_dirty = std::env::args().any(|c| c == "--allow-dirty");

    let manifest_path = manifest_path();

    let mut metadata_command = cargo_metadata::MetadataCommand::new();
    metadata_command.no_deps();
    let mut other_options = vec!["--locked".to_owned()];

    if let Some(manifest_path) = &manifest_path {
        other_options.extend_from_slice(&["--manifest-path".to_owned(), manifest_path.to_owned()]);
    }
    metadata_command.other_options(other_options);
    let metadata = metadata_command
        .exec()
        .expect("Failed to get project metadata");
    let target_directory = &metadata.target_directory;
    let package_flag = package_flag();
    let package_to_publish = if let Some(package_flag) = package_flag {
        metadata
            .packages
            .iter()
            .find(|p| p.name.as_str() == package_flag)
            .unwrap_or_else(|| panic!("No package with name `{package_flag}` found"))
    } else if metadata.packages.len() == 1 {
        &metadata.packages[0]
    } else {
        let check_path = manifest_path
            .map(PathBuf::from)
            .and_then(|p| p.parent().map(|p| p.to_owned()))
            .unwrap_or_else(|| std::env::current_dir().unwrap());
        // necessary to allow relative paths
        let check_path = check_path.clone().canonicalize().unwrap_or(check_path);
        metadata
            .packages
            .iter()
            .find(|p| p.manifest_path.parent().unwrap() == check_path)
            .unwrap_or_else(|| panic!("Could not identify package to publish"))
    };
    let package_root = package_to_publish.manifest_path.parent().unwrap();
    let package_version = &package_to_publish.version;
    let package_name = &package_to_publish.name;
    println!(
        "Run cargo safe-publish for the crate `{package_name} {package_version} ({package_root})`",
    );

    if !is_allow_dirty {
        check_git_is_dirty(package_root);
    }

    if !is_no_verify {
        run_verification_build(
            target_directory.as_std_path(),
            package_name.as_str(),
            package_version,
        );
    }

    if !is_dry_run && !is_help {
        run_publish();

        let everything_matched =
            verify_content_matches(package_root, package_version, package_name.as_str());
        if everything_matched {
            println!();
            println!("Successfully published and verified `{package_name}` ({package_version})");
        } else {
            eprintln!();
            eprintln!(
                "{}: Found a difference between the uploaded and the local version. \
                 Double check if thats desired, otherwise please yank \
                 version {package_version} of `{package_name}`",
                "error".red().bold()
            );
            std::process::exit(1);
        }
    }
}
