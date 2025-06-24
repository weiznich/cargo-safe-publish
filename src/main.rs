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

use std::io::Read;
use std::process::{Command, Stdio};

use flate2::read::GzDecoder;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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

fn main() {
    let is_dry_run = std::env::args().any(|c| c == "--dry-run");
    let is_no_verify = std::env::args().any(|c| c == "--no-verify");
    let is_help = std::env::args().any(|c| c == "--help" || c == "-h");

    let manifest_path = manifest_path();

    let mut metadata_command = cargo_metadata::MetadataCommand::new();
    metadata_command.no_deps();
    let mut other_options = vec!["--locked".to_owned()];

    if let Some(manifest_path) = manifest_path {
        other_options.extend_from_slice(&["--manifest_path".into(), manifest_path]);
    }
    metadata_command.other_options(other_options);
    let metadata = metadata_command
        .exec()
        .expect("Failed to get project metadata");
    let package_flag = package_flag();
    let package_to_publish = if let Some(package_flag) = package_flag {
        metadata
            .packages
            .iter()
            .filter_map(|p| (p.name.as_str() == package_flag).then_some(p))
            .next()
            .unwrap_or_else(|| panic!("No package with name `{package_flag}` found"))
    } else if metadata.packages.len() == 1 {
        &metadata.packages[0]
    } else {
        let current_dir = std::env::current_dir().unwrap();
        metadata
            .packages
            .iter()
            .filter_map(|p| (p.manifest_path.parent().unwrap() == current_dir).then_some(p))
            .next()
            .unwrap_or_else(|| panic!("Could not identify package to publish"))
    };
    let package_root = package_to_publish.manifest_path.parent().unwrap();
    let package_version = &package_to_publish.version;
    let package_name = &package_to_publish.name;
    println!(
        "Run cargo safe-publish for the crate `{package_name} {package_version} ({package_root})`",
    );

    if !is_no_verify {
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
                eprintln!("Dry run failed: {e}");
                std::process::exit(1);
            }
            Ok(s) if !s.success() => {
                eprintln!(
                    "Dry run returned a non-zero exist code, check the output above for details"
                );
                std::process::exit(1);
            }
            Ok(_) => {}
        }
    }

    if !is_dry_run && !is_help {
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
                eprintln!("Dry run failed: {e}");
                std::process::exit(1);
            }
            Ok(s) if !s.success() => {
                eprintln!(
                    "Dry run returned a non-zero exist code, check the output above for details"
                );
                std::process::exit(1);
            }
            Ok(_) => {}
        }

        let body = ureq::get(format!(
            "https://crates.io/api/v1/crates/{package_name}/{package_version}/download"
        ))
        .header("User-Agent", format!("cargo-safe-publish/{APP_VERSION}"))
        .call()
        .expect("Failed to fetch package")
        .body_mut()
        .read_to_vec()
        .expect("Failed to fetch package");

        let zipped_archive = GzDecoder::new(std::io::Cursor::new(body));
        let mut archive = tar::Archive::new(zipped_archive);
        for entry in archive
            .entries()
            .expect("Could not open uploaded `.crate` archive")
        {
            let mut entry = entry.expect("Failed to get file entry from tar archive");

            let path = entry.path().unwrap();
            let local_path = package_root.join(path.display().to_string());
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
                        "Found a difference between the uploaded and the local version. \
                                   Double check if thats desired, otherwise please yank \
                                   version {package_version} of `{package_name}`"
                    );
                    eprintln!("{diff}");
                    std::process::exit(1);
                }
            } else {
                eprintln!(
                    "The file `{path}` does not exist in `{package_root}`. \
                         It seems to be added by the publication process. \
                         Double check if thats desired, otherwise please yank \
                         version {package_version} of `{package_name}`",
                    path = path.display()
                );
                std::process::exit(1);
            }
        }
    }
}
