# Cargo Safe-Publish

A safer version of `cargo publish`.

`cargo safe-publish` performs the following steps to make your publication process more secure:

* `cargo safe-publish` uses the [`gix`](https://crates.io/crates/gix) to perform additionally git checks to verify that only expected files are included in your published crate
* `cargo safe-publish` split up the actual publication process into a call to `cargo publish --dry-run` and `cargo publish --no-verify`. The former command performs the verification build to make sure that the published source code is actually be able compile. After this `cargo` aborts the publication process. `cargo safe-publish` then removes the compressed `.crate` file. Finally `cargo publish --no-verify` will recreate the compressed `.crate` file and upload it without a verification build. This removes the possibility for build scripts to overwrite that file.
* `cargo safe-publish` re-downloads the published crate, right after the publication process and compares the published content. It will report any difference it detect

See [the announcement blog post](https://blog.weiznich.de//cargo-safe-publish/) for details.

## License

Licensed under [GPL-2 or later](https://www.gnu.org/licenses/old-licenses/gpl-2.0.html)
