# Releasing Brume

## The short version

```bash
pwsh tools/new-release.ps1 -Version 0.2.0 -Notes "What changed in this release."
```

That bumps, builds, signs and writes `dist/latest.json`, then stops and prints the command to
publish. Add `-Publish` to go all the way, including creating the GitHub release.

Publishing is opt-in rather than the default because it is outward-facing and one-way: once a
release is public, anyone running Brume may download it on next launch.

---

## What a release actually consists of

Three artifacts, doing three different jobs. Confusing them is the easiest way to ship a broken
release.

| Artifact | Who consumes it |
|---|---|
| `Brume-Setup.exe` | **Humans.** The custom-UI installer, for a first install. |
| `Brume_<ver>_x64-setup.exe` | **The updater.** The plain NSIS installer, downloaded in the background. |
| `Brume_<ver>_x64-setup.exe.sig` | **The updater.** Detached signature for the above. |
| `latest.json` | **The updater.** The manifest that says a new version exists. |

Both `.exe` files are installers, which is confusing until you know why: the first is the styled
shell a person double-clicks, the second is what it wraps. Updates use the inner one directly.

> Tauri 2.11 signs the installer `.exe` itself. Older versions wrapped it in a `.nsis.zip`
> first, and a lot of documentation still says so. If a future upgrade reintroduces the archive,
> `tools/new-release.ps1` is where the filename pattern lives.

Note what is *not* in the update path: the pretty installer shell. Updates run the NSIS
installer directly, passively, with no UI. That is by design — see
[INSTALLER.md](INSTALLER.md).

---

## The signing key

### Where it is

```
%USERPROFILE%\.tauri\brume-updater.key       <- private. The secret.
%USERPROFILE%\.tauri\brume-updater.pass      <- its password.
%USERPROFILE%\.tauri\brume-updater.key.pub   <- public. Already in tauri.conf.json.
```

Deliberately **outside the repository**, so they cannot be committed by accident. `.gitignore`
also blocks `*.key`, but that is a backstop, not the control. Both files are ACL'd to your user
account with inheritance removed.

Together they are the most damaging thing that could leak from this project: anyone holding
them can sign a payload that **every existing Brume install will accept as a legitimate update
and execute**.

### Why the key must have a password on Windows

This is not a preference. Tauri prompts for the key password on stdin unless
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is set — and **Windows cannot represent an empty
environment variable.** Both `$env:X = ''` and `[Environment]::SetEnvironmentVariable(X, '')`
delete the variable outright, and the child process sees nothing.

So a passwordless key cannot be used in an unattended build at all. The build compiles, bundles,
reaches the signing step, and then hangs forever on a prompt no one can answer — *after*
everything appeared to succeed. It looks exactly like a stalled build.

`tools/build-installer.ps1` now fails fast with an explanation if the key is present but the
password is not, rather than letting the build reach that prompt.

### Rotating the key

```bash
npx tauri signer generate -w "$env:USERPROFILE\.tauri\brume-updater.key" -p <password> -f --ci
```

Then put the new public key into `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` and
save the new password to `brume-updater.pass`. **Read the next section first** — rotating after
anything has shipped strands every existing install.

### ⚠️ Losing this key is unrecoverable

The public key is compiled into every copy of Brume that ships. A client only accepts updates
signed by the key matching the public key *it* carries. So if the private key is lost:

- You can generate a new keypair and ship new installers.
- Every install already in the wild will reject every future update, silently, forever.
- The only fix is asking each user to manually download and reinstall.

Back it up somewhere you would not lose — a password manager's secure-note field is a
reasonable home for it.

### For CI

Set these as repository secrets and let the build read them from the environment.
`tools/build-installer.ps1` already prefers the environment over the local file, so CI needs no
code change:

| Variable | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | The **contents** of the `.key` file, not a path |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The contents of the `.pass` file |

Both are required — see above for why the password cannot be omitted. Never commit either.

---

## How an update actually reaches someone

1. Brume launches. If `auto_update` is on, it fetches the endpoint configured in
   `src-tauri/tauri.conf.json`:

   ```
   https://github.com/London-Christensen/brume-browser/releases/latest/download/latest.json
   ```

   `releases/latest/download/<asset>` is a GitHub redirect to that asset on the newest
   published release, which is why the manifest never needs a hardcoded version in its URL.

2. If `latest.json` advertises a version higher than the running one, Brume shows a prompt with
   the version number and release notes. **It never installs silently.**

3. On confirm, it downloads the `.nsis.zip`, verifies the signature against the compiled-in
   public key, and runs the installer passively.

4. Windows cannot replace a running executable, so the app exits during install and reopens
   afterwards. The prompt says so before it happens, rather than appearing to vanish.

### The repository must stay public

The endpoint is an unauthenticated GitHub URL. Releases on a private repository are private
too, so making this repo private breaks auto-update for everyone — the request 404s and the
check fails silently. If it ever needs to be private, the manifest and artifacts have to move
to some other public host.

### There is no update until there is a second release

With only `v0.1.0` published, a `v0.1.0` client checks, finds `0.1.0`, and correctly concludes
it is up to date. Auto-update cannot be meaningfully tested until a *newer* release exists —
which is what step 10 of the build plan does.

---

## What `new-release.ps1` does, and why

### 1. Refuses to build from a dirty tree

A release you cannot reproduce from a commit is not a release. `-SkipDirtyCheck` exists for
experiments.

### 2. Bumps the version in five files

`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`,
`installer-shell/Cargo.toml`, `installer-shell/tauri.conf.json`.

They are updated together and then re-read and verified, because a mismatch **does not fail the
build**. It produces an installer and a manifest that quietly disagree, and updates simply stop
working with no error anywhere.

### 3. Builds and signs

Via `tools/build-installer.ps1`, which also runs `cargo clean -p brume` first — see the
bundle-type marker note in [BUILD_NOTES.md](BUILD_NOTES.md) for why that is not optional.

### 4. Writes `dist/latest.json`

```json
{
  "version": "0.2.0",
  "notes": "What changed.",
  "pub_date": "2026-07-30T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of the .sig file>",
      "url": "https://github.com/London-Christensen/brume-browser/releases/download/v0.2.0/Brume_0.2.0_x64-setup.exe"
    }
  }
}
```

The download URL is pinned to the **tag**, not to `/latest`. A `/latest` URL inside the manifest
would make every historical release advertise whichever build happens to be newest at download
time, which is not what any of them promised.

---

## Doing it by hand

If the script is unavailable or you want to understand each step:

```bash
# 1. Bump the version in all five files listed above.

# 2. Build and sign.
pwsh tools/build-installer.ps1

# 3. Collect from src-tauri/target/release/bundle/nsis/
#      Brume_<ver>_x64-setup.exe
#      Brume_<ver>_x64-setup.exe.sig

# 4. Write latest.json in the shape above, pasting the .sig contents verbatim
#    into "signature".

# 5. Tag and publish.
git commit -am "Release 0.2.0"
git tag v0.2.0
git push --follow-tags

gh release create v0.2.0 --title "Brume 0.2.0" --notes "..." \
  dist/Brume-Setup.exe \
  src-tauri/target/release/bundle/nsis/Brume_0.2.0_x64-setup.exe \
  src-tauri/target/release/bundle/nsis/Brume_0.2.0_x64-setup.exe.sig \
  dist/latest.json
```

`latest.json` **must** be attached to the release as an asset — that is what the endpoint URL
resolves to.

---

## Next step: automate it with GitHub Actions

Not built yet, deliberately. The manual path should be understood before it is hidden behind
CI, and there is no point automating a release process that is still changing shape.

When it is worth doing, the shape is:

- Trigger on pushing a `v*` tag.
- Runner: `windows-latest`.
- Install Rust and Node, run `tools/build-installer.ps1`.
- Inject `TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD` from repository secrets.
- Generate `latest.json` and upload everything with `softprops/action-gh-release` or `gh`.

`tauri-apps/tauri-action` does most of this off the shelf, though it assumes a standard Tauri
layout and would need coaxing to also build the installer shell in stage 2.

The main thing CI buys here is not speed but reproducibility: builds stop depending on one
laptop having the right toolchain and the right key.

---

## Troubleshooting

| Symptom | Cause |
|---|---|
| Build warns about no signing key | `TAURI_SIGNING_PRIVATE_KEY` unset and no key file. Artifacts will be unsigned and rejected by clients. |
| Build appears to hang after "Finished 1 bundle" | It is waiting on the key password prompt. Look for `Info Decrypting updater signing key, expect a prompt for password`. Restore `brume-updater.pass` or set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. |
| `new-release.ps1` errors that no `.nsis.zip` was produced | `bundle.createUpdaterArtifacts` is not `true`, or signing failed. |
| Clients never see the update | `latest.json` not attached to the release; repo went private; or the version in the manifest is not higher than the client's. |
| Clients see it but installation fails | Signature mismatch — the artifact was signed with a different key than the public key compiled into that client. |
| `Failed to add bundler type to the binary` during build | The binary was not relinked. See [BUILD_NOTES.md](BUILD_NOTES.md); the package may refuse to update. |
