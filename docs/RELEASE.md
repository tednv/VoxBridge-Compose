# Release process

VoxBridge Compose releases are built from a version tag by
`.github/workflows/release.yml`.

## Prepare

1. Update the same semantic version in `package.json`, `src-tauri/Cargo.toml`,
   and `src-tauri/tauri.conf.json`.
2. Confirm the VoxBridge submodule points to a public commit and initialize it
   recursively in a clean checkout.
3. Run `npm ci`, `npm run build`, and the applicable native checks.
4. Review the complete outgoing diff, dependency licenses, commit ancestry, and
   release text. Exclude local paths, logs, private configuration, session notes,
   credentials, and generated model or recording data.
5. Test the locally built Windows artifacts. Test Linux artifacts on a supported
   distribution before describing Linux support as stable.

## Publish

Create and push an annotated `vX.Y.Z` tag only after the release commit is on
`main`. The workflow builds Windows and Linux packages, generates SHA-256
checksums, and creates the GitHub release.

After publication, verify the public repository, release notes, asset names,
checksums, source archive, submodule links, and installer metadata from the
public view.
