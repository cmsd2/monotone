# Releasing monotone

The workspace holds the `monotone` library and the `monotone-cli` binary. Both
crates share one version, set once in the root `Cargo.toml` under
`[workspace.package]`.

Releases run from a tag through `.github/workflows/release.yml`. The workflow
checks that the tag matches the manifest version, runs CI, builds binaries for
Linux x86_64, macOS aarch64 and macOS x86_64, publishes `monotone` then
`monotone-cli` to crates.io, and creates a GitHub release with the binaries.

## One-time setup

Add a crates.io API token with publish rights for both crates as the repository
secret `CARGO_REGISTRY_TOKEN`.

## Steps

1. Set the new version in the root `Cargo.toml` under `[workspace.package]`.
   For a minor or major bump, also update the `monotone` requirement under
   `[workspace.dependencies]`.
2. Update version numbers in `README.md`.
3. Run `cargo update -w` so `Cargo.lock` records the new version.
4. Commit and merge to `master` with CI green.
5. Optionally rehearse with a pre-release tag. It runs everything except the
   publish, which becomes a dry run, and it creates no GitHub release:

   ```sh
   git tag v0.5.1-rc.1 && git push origin v0.5.1-rc.1
   ```

6. Tag the release and push the tag:

   ```sh
   git tag v0.5.1 && git push origin v0.5.1
   ```

7. Watch the Release workflow under Actions. If the version check fails, fix the
   manifests, delete the tag locally and on GitHub, and tag again.

crates.io publishes cannot be undone. If a release is broken, fix it and
release the next patch version.
