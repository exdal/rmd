# Rapid Mapping Device

A map editor for Space Station 13.

### demir

demir is a collection of my many personal compiler projects. It is a compiler frontend and hopefully one day intermediate representation for DreamMaker langauage, thus it's name: **d**r**e**a**m** **IR**.

<sub>demir is Turkish; it means iron. Both words contain "ir" in them, it was one of the reasons I choose demir for this project, again.</sub>

## Development

The workspace uses the Rust toolchain declared in `rust-toolchain.toml` and requires the Slang shader compiler while building.
Before submitting a change, run:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

GitHub Actions also builds release-mode editor binaries for Windows and Linux. These are available as temporary artifacts on each CI run.

## Releases

The editor version in `rmd/editor/Cargo.toml` is the release version. After updating it and `Cargo.lock`, merge the change, tag that commit, and push the tag:

```console
git tag v1.3.0
git push origin v1.3.0
```

The tag must have the form `vMAJOR.MINOR.PATCH` and match the editor version. The release workflow reruns CI, builds both platforms, generates notes from the non-merge commits since the previous version tag, and publishes these assets:

```text
rmd-1.3.0-x86_64-pc-windows-msvc.exe
rmd-1.3.0-x86_64-unknown-linux-gnu
```

Linux users may need to make the downloaded binary executable with `chmod +x rmd-<version>-x86_64-unknown-linux-gnu`.
