# Keychain access prompts

Since v0.3, Brainiac keeps GitHub and Bitbucket Cloud account tokens in the
macOS Keychain (`SPEC.md`, Accounts; `docs/architecture.md`, Pull requests —
v0.3). Each one is a generic password with service `brainiac` and account
`github` or `bitbucket`. This page explains why macOS may ask
*"brainiac wants to use your confidential information stored in "brainiac" in
your keychain"* every time the app starts, and how to stop it.

## Why macOS keeps asking

A Keychain item has an access list naming the programs that can read it
without asking. You can see it with:

```sh
security dump-keychain -a ~/Library/Keychains/login.keychain-db \
  | grep -A30 '"svce"<blob>="brainiac"' | grep requirement
```

For a build that is not signed with a certificate, the list looks like this:

```
requirement: cdhash H"<40 hex digits>"                       # one exact binary
requirement: identifier "com.apple.security" and anchor apple  # the `security` CLI
```

- **Unsigned builds are known only by their hash.** `pnpm tauri dev` binaries
  are signed by the linker with no identity (ad hoc), and so is a release built
  without the Apple secrets (`codesign -dv` shows `Signature=adhoc`). With no
  certificate, the only way macOS can recognize the program is its code
  directory hash (cdhash).
- **"Always Allow" trusts that hash and nothing else.** Every rebuild produces
  a new binary with a new hash, so macOS treats the next build as a different
  program and asks again. If you rebuild often, that means a prompt on nearly
  every launch.
- **It shows up at startup** because the first pull request request
  (`PullRequestService::session`) reads the token. `AccountService` keeps the
  token in memory after that, so you get one prompt per run for each provider,
  not one per request.
- **An item made with `security add-generic-password`** trusts only the
  `security` tool, so even a properly signed build is asked once before it
  can read the item.

The same thing happens to users: if releases are not signed, each update ships
a new hash, and users get the prompt again after every update.

## Fix: sign with an identity that stays the same

Once a build is signed with a certificate, its designated requirement becomes
"this identifier, signed by this certificate". That stays the same across
builds, so one "Always Allow" lasts.

### Local development: a self-signed certificate (free)

1. Open **Keychain Access** → *Keychain Access* menu → **Certificate
   Assistant** → **Create a Certificate…**
   - Name: `Brainiac Dev` (any name works; it stays on your machine)
   - Identity Type: **Self Signed Root**
   - Certificate Type: **Code Signing**
2. Trust it for code signing. Certificate Assistant leaves the new certificate
   untrusted, so the next command reports `0 valid identities found` until you
   do this. In Keychain Access, select the **login** keychain and **My
   Certificates**, double-click **Brainiac Dev**, open **Trust**, and set
   **Code Signing** to **Always Trust**. Close the window and authenticate.
   Without `-v`, the same certificate shows up as `CSSMERR_TP_NOT_TRUSTED`.
3. Check that it is available for signing:

   ```sh
   security find-identity -v -p codesigning
   ```

4. Sign a packaged build with it:

   ```sh
   APPLE_SIGNING_IDENTITY="Brainiac Dev" pnpm tauri build
   ```

   Tauri signs the `.app` with that identity.
5. `pnpm tauri dev` and `pnpm tauri:dev` run the bare binary from `target/`
   without bundling it, so Tauri does not sign it. On macOS those commands
   install `scripts/codesign-run.sh` as the Cargo
   [runner](https://doc.rust-lang.org/cargo/reference/config.html#targettriplerunner).
   It signs the binary before starting it when `BRAINIAC_CODESIGN_IDENTITY`
   is set, and leaves the binary untouched when the variable is unset, so
   nothing specific to one machine is committed and CI stays unsigned:

   ```sh
   BRAINIAC_CODESIGN_IDENTITY="Brainiac Dev" pnpm tauri dev
   ```

   The signature uses the identifier `dev.brainiac.desktop`, the same one as
   a packaged build.
6. Start the signed build and choose **Always Allow** once more. Later rebuilds
   signed with the same certificate and identifier will not ask again.

A self-signed certificate is trusted only on the machine that made it.
Gatekeeper still treats such builds as unidentified, so this does not help
distribution.

### Releases: a Developer ID certificate

Signing releases needs a Developer ID Application certificate, which comes with
the Apple Developer Program (a paid membership).
`.github/workflows/release.yml` already uses one when the `APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
`APPLE_PASSWORD`, and `APPLE_TEAM_ID` secrets are configured (see
`docs/RELEASING.md`). With it:

- every release is signed by the same team, so users approve Keychain access
  once and it lasts through updates;
- the app is notarized, so first-time users no longer need to right-click →
  Open.

## Options that do not fit

- **The Data Protection Keychain** (`kSecUseDataProtectionKeychain`) never
  shows these prompts, but it needs a `keychain-access-groups` entitlement,
  and that needs a team ID and a provisioning profile, which means the paid
  program again. It does not help unsigned builds.
- **Keeping tokens in a file or in `brainiac.db`** would go against the
  accepted decision that tokens are kept only in the Keychain
  (`docs/architecture.md`, Decisions).

## Making the prompt less intrusive

Signing removes the cause. On top of that, the app could avoid reading a token
at launch and wait until it is first needed, such as when a Pull requests tab
opens or the first scheduled refresh runs. That does not remove the prompt for
unsigned builds, but it means a launch that never touches pull requests does
not ask at all.
