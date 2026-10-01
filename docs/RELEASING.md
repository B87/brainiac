# Releasing Brainiac

Releases are built by GitHub Actions from a `vX.Y.Z` tag and distributed as a
GitHub release. Installed copies of the app check that release feed and can
update themselves. This page is the whole process, including the one-time setup.

## How it fits together

1. `scripts/release.sh X.Y.Z` bumps the version in `package.json`,
   `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, commits, tags
   `vX.Y.Z` and pushes.
2. `.github/workflows/release.yml` wakes on the tag, checks that the tag and the
   manifests agree, builds a universal macOS bundle, signs the updater artifact,
   and opens a **draft** release containing:
   - `Brainiac_X.Y.Z_universal.dmg` for people installing by hand,
   - `Brainiac.app.tar.gz` and `Brainiac.app.tar.gz.sig` for the in-app updater,
   - `latest.json`, the manifest the updater reads.
   The release notes are the matching section of `CHANGELOG.md`.
3. You download the DMG from the draft, try it, then run
   `scripts/publish-release.sh vX.Y.Z`. Publishing is the moment
   `https://github.com/B87/brainiac/releases/latest/download/latest.json`
   starts pointing at the new build, so every installed app sees it on its next
   check.

The updater only accepts artifacts whose signature verifies against the public
key embedded in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). A
compromised GitHub account cannot push an update to users without the private
key as well.

## One-time setup

### 1. Create the GitHub repository

```sh
gh repo create B87/brainiac --public --source=. --remote=origin
```

If you pick another owner or name, change the `endpoints` URL in
`src-tauri/tauri.conf.json`, the links at the bottom of `CHANGELOG.md`, and this
page.

### 2. Signing key

The key pair was generated with `pnpm tauri signer generate` and lives outside
the repository:

- private key: `~/.tauri/brainiac.key` (never commit it; `.gitignore` blocks `*.key`)
- public key: `~/.tauri/brainiac.key.pub`, already copied into `tauri.conf.json`

Back the private key up somewhere safe (a password manager is fine). **If it is
lost, installed apps can never update again**: a new key means a new public key
in the config, which only ships with a build the old key can no longer sign.
Users would have to reinstall by hand once.

Store it in the repository secrets:

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/brainiac.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --body ""
```

The key was generated without a password (`--ci`). If you want one, regenerate
with `pnpm tauri signer generate -w ~/.tauri/brainiac.key -f`, update the public
key in `tauri.conf.json`, and set the password secret.

### 3. Apple Developer signing (optional, recommended later)

Without it the bundle is ad-hoc signed. Gatekeeper then blocks the first launch
of a downloaded DMG until the user right-clicks the app and chooses Open. The
in-app updater is not affected: it replaces the bundle itself, so the
downloaded files never carry the quarantine attribute.

When you have an Apple Developer account, add these secrets and the workflow
starts signing and notarizing automatically:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_PASSWORD` | an app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | the ten-character team id |

## Cutting a release

1. Move the entries under `## [Unreleased]` in `CHANGELOG.md` into a new
   `## [X.Y.Z] - YYYY-MM-DD` section and add the comparison link at the bottom.
   Commit that on `main`.
2. Run the full check suite: `pnpm check`.
3. Cut it:

   ```sh
   scripts/release.sh X.Y.Z --dry-run   # shows the version bump without committing
   scripts/release.sh X.Y.Z             # commit, tag, push
   gh run watch                          # follow the build (about 10 minutes)
   ```

4. Open the draft (`gh release view vX.Y.Z --web`), download the DMG, install
   it over your current copy and use it for a minute.
5. Publish: `scripts/publish-release.sh vX.Y.Z`. The script refuses if any of
   the updater assets are missing.
6. Pick Brainiac > Check for Updates… in an older installed build to confirm the
   update is offered, installs, and relaunches.

Pre-releases work the same way with a tag like `v0.2.0-beta.1`. They are marked
as pre-release on GitHub and are not served from `releases/latest`, so regular
users do not see them.

## Local release build

`pnpm tauri build --target universal-apple-darwin` produces the same bundle the
workflow does (install the Intel target first with
`rustup target add x86_64-apple-darwin`). To also produce the signed updater
artifact locally, export the key first:

```sh
export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/brainiac.key)
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
pnpm tauri build --target universal-apple-darwin
```

## Testing the updater without publishing

Point the app at a local manifest instead of GitHub: temporarily change
`plugins.updater.endpoints` in `tauri.conf.json` to
`http://localhost:8000/latest.json` and add `"dangerousInsecureTransportProtocol": true`
next to it, build with the signing key exported, serve the directory holding
`latest.json`, `Brainiac.app.tar.gz` and its `.sig` with `python3 -m http.server`,
then run an older build. Revert both config changes before committing.
