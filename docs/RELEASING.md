# Releasing Brume

## The short version

```bash
powershell tools/new-release.ps1 -Version 0.2.0 -Notes "What changed in this release."
```

That bumps, builds, signs and writes `dist/latest.json`, then stops and prints the command to
publish. Add `-Publish` to go all the way, including creating the GitHub release.

Publishing is opt-in rather than the default because it is outward-facing and one-way: once a
release is public, anyone running Brume may download it on next launch.

---

## What a release actually consists of

Three artifacts, doing three different jobs. Confusing them is the easiest way to ship a broken
release.

| Artifact | Who consumes it | Where it goes |
|---|---|---|
| `Brume-Setup.exe` | **Humans** on a first install, **and the updater**. | The release page |
| `latest.json` | **The updater.** Says a newer version exists and where. | `updates/` in the repo |
| `Brume_<ver>_x64-setup.exe` | Nothing directly. Embedded inside `Brume-Setup.exe`. | Nowhere |
| `Brume-Setup.exe.sig` | Nothing. Its contents are copied into `latest.json`. | Nowhere |

**One file is published, and it is the one a person should click.**

That is deliberate. The release page used to list four things when only one of
them was meant for a human, with nothing to indicate which. Two changes fixed it:

1. **`latest.json` is served from the repository**, at
   `raw.githubusercontent.com/.../main/updates/latest.json`, rather than being
   attached to the release.
2. **`Brume-Setup.exe` is the update payload as well as the human download.** It
   already carries the NSIS installer inside it, so publishing that separately
   was shipping a second copy of a file that was already there.

The updater runs whatever it downloads with `/P /R /UPDATE /ARGS`. It decides an
`.exe` is an NSIS installer by sniffing the file's contents, not its name, so
`Brume-Setup.exe` qualifies. `installer-shell/src/main.rs` watches for those
flags and hands straight to the installer it carries instead of drawing its UI.

The cost is that an update downloads about 5 MB rather than 1.9 MB, because the
shell and its assets come along with the payload.

GitHub attaches **Source code (zip)** and **Source code (tar.gz)** to every
release automatically and there is no way to suppress them, so a release page
shows three entries in total.

### The signature covers Brume-Setup.exe now

Tauri signs the NSIS installer automatically, but that is no longer the file
being advertised. The signature in the manifest has to cover the exact bytes a
client downloads, so `new-release.ps1` signs `Brume-Setup.exe` itself with
`npx tauri signer sign` after stage 2 and reads that `.sig` into the manifest.
`bundle.createUpdaterArtifacts` stays enabled regardless, because it is also
what stamps the bundle-type marker the updater reads at runtime.

### The one-release bridge for old installs

Installs from 0.2.0 and earlier have the previous endpoint compiled in:

```
https://github.com/London-Christensen/brume-browser/releases/latest/download/latest.json
```

They cannot be told the address changed. So the **first** release under the new
scheme is published with `-AttachLegacyFeed`, which attaches `latest.json` to it
as well:

```bash
powershell tools/new-release.ps1 -Version 0.3.0 -Notes "..." -Publish -AttachLegacyFeed
```

An old install resolves that URL against the newest release, finds the manifest,
updates to the new version, and reads from the repository from then on. That
release page carries two files instead of one, for that release only.

**Do not pass it again.** Once a later release omits the asset the old URL 404s,
which is harmless because nothing is still pointing at it.

### Order matters when publishing

`latest.json` names a download URL on a release that has to exist first. The
script therefore creates the GitHub release, and only then commits and pushes
`updates/latest.json`. Doing it the other way round advertises a version whose
installer is not there yet, and every client checking in that window reports a
failed download.

Both files are installers, which is confusing until you know why: `Brume-Setup.exe` is the styled
shell a person double-clicks, and the NSIS installer is what it wraps. Only the shell is
published, because it contains the other one.

> Tauri 2.11 signs the installer `.exe` itself. Older versions wrapped it in a `.nsis.zip`
> first, and a lot of documentation still says so. If a future upgrade reintroduces the archive,
> `tools/new-release.ps1` is where the filename pattern lives.

An update still ends up running NSIS passively with no UI, but it gets there through the shell
rather than around it: the updater runs `Brume-Setup.exe`, which sees `/UPDATE` and hands over to
the installer it carries without drawing anything. See [INSTALLER.md](INSTALLER.md).

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
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is set, and **Windows cannot represent an empty
environment variable.** Both `$env:X = ''` and `[Environment]::SetEnvironmentVariable(X, '')`
delete the variable outright, and the child process sees nothing.

So a passwordless key cannot be used in an unattended build at all. The build compiles, bundles,
reaches the signing step, and then hangs forever on a prompt no one can answer, *after*
everything appeared to succeed. It looks exactly like a stalled build.

`tools/build-installer.ps1` now fails fast with an explanation if the key is present but the
password is not, rather than letting the build reach that prompt.

### Rotating the key

```bash
npx tauri signer generate -w "$env:USERPROFILE\.tauri\brume-updater.key" -p <password> -f --ci
```

Then put the new public key into `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` and
save the new password to `brume-updater.pass`. **Read the next section first.** Rotating after
anything has shipped strands every existing install.

### Losing this key is unrecoverable

The public key is compiled into every copy of Brume that ships. A client only accepts updates
signed by the key matching the public key *it* carries. So if the private key is lost:

- You can generate a new keypair and ship new installers.
- Every install already in the wild will reject every future update, silently, forever.
- The only fix is asking each user to manually download and reinstall.

Back it up somewhere you would not lose; a password manager's secure-note field is a
reasonable home for it.

### For CI

Set these as repository secrets and let the build read them from the environment.
`tools/build-installer.ps1` already prefers the environment over the local file, so CI needs no
code change:

| Variable | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | The **contents** of the `.key` file, not a path |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The contents of the `.pass` file |

Both are required; see above for why the password cannot be omitted. Never commit either.

---

## How an update actually reaches someone

1. Brume launches. If `auto_update` is on, it fetches the endpoint configured in
   `src-tauri/tauri.conf.json`:

   ```
   https://raw.githubusercontent.com/London-Christensen/brume-browser/main/updates/latest.json
   ```

   A plain file in the repository. **This URL is compiled into every shipped copy and can
   never change**: an install already in the wild keeps asking this exact address forever, so
   moving or renaming the file strands it as surely as losing the signing key would.

2. If `latest.json` advertises a version higher than the running one, Brume shows a prompt with
   the version number and release notes. **It never installs silently.**

3. On confirm, it downloads `Brume-Setup.exe` from the release the manifest names and verifies
   the signature against the compiled-in public key.

4. It runs that file with `/P /R /UPDATE /ARGS`. `Brume-Setup.exe` sees `/UPDATE`, skips its own
   UI, and hands the same flags to the NSIS installer it carries. See
   `installer-shell/src/main.rs`.

5. Windows cannot replace a running executable, so the app exits during install and reopens
   afterwards. The prompt says so before it happens, rather than appearing to vanish.

### The repository must stay public

The endpoint is an unauthenticated GitHub URL. Releases on a private repository are private
too, so making this repo private breaks auto-update for everyone: the request 404s and the
check fails silently. If it ever needs to be private, the manifest and artifacts have to move
to some other public host.

### There is no update until there is a second release

With only `v0.1.0` published, a `v0.1.0` client checks, finds `0.1.0`, and correctly concludes
it is up to date. Auto-update cannot be meaningfully tested until a *newer* release exists,
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

Via `tools/build-installer.ps1`, which also runs `cargo clean -p brume` first; see the
bundle-type marker note in [BUILD_NOTES.md](BUILD_NOTES.md) for why that is not optional.

### 4. Signs `Brume-Setup.exe` and writes the manifest

```json
{
  "version": "0.2.0",
  "notes": "What changed.",
  "pub_date": "2026-07-30T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of dist/Brume-Setup.exe.sig>",
      "url": "https://github.com/London-Christensen/brume-browser/releases/download/v0.2.0/Brume-Setup.exe"
    }
  }
}
```

Written to `dist/latest.json` first. The copy that clients actually read is
`updates/latest.json`, committed after the release exists.

The download URL is pinned to the **tag**, not to `/latest`. A `/latest` URL inside the manifest
would make every historical release advertise whichever build happens to be newest at download
time, which is not what any of them promised.

---

## Doing it by hand

If the script is unavailable or you want to understand each step:

```bash
# 1. Bump the version in all five files listed above.

# 2. Build and sign.
powershell tools/build-installer.ps1

# 3. Sign the file that will be published. Tauri signed the NSIS installer, but
#    that one is only embedded, so its signature is not the one clients check.
npx tauri signer sign dist/Brume-Setup.exe

# 4. Write latest.json in the shape above, pasting dist/Brume-Setup.exe.sig
#    verbatim into "signature". Do not edit it afterwards: the signature covers
#    exact bytes and a mismatch is rejected silently.

# 5. Tag and publish. The tag must be annotated (-a): git push --follow-tags
#    silently ignores lightweight ones, leaving it local, and gh then refuses to
#    build a release from a tag the remote has never seen.
git commit -am "Release 0.2.0"
git tag -a v0.2.0 -m "Brume 0.2.0"
git push --follow-tags

gh release create v0.2.0 --title "Brume 0.2.0" --notes "..." \
  dist/Brume-Setup.exe

# 6. LAST, and only once the release exists. The manifest names a URL on it, so
#    pushing this first advertises an installer that is not there yet.
cp dist/latest.json updates/latest.json
git add updates/latest.json
git commit -m "Point the update feed at 0.2.0"
git push
```

`updates/latest.json` is **not** a release asset. It is a tracked file, served over
`raw.githubusercontent.com`, and that path is compiled into every shipped copy.

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
| Build fails with `expected value at line 1 column 1` | A UTF-8 BOM in a config file. `Set-Content -Encoding utf8` adds one on PowerShell 5.1 and strict parsers reject it; the message names neither the file's real problem nor the tool that caused it. The bump now writes through `UTF8Encoding(false)` and checks afterwards. |
| `gh release create` says the tag "has not been pushed" | The tag was lightweight. `git push --follow-tags` only pushes **annotated** tags, so it stayed local. Use `git tag -a`, or push the tag by name. |
| `new-release.ps1` errors that no `.nsis.zip` was produced | `bundle.createUpdaterArtifacts` is not `true`, or signing failed. |
| Clients never see the update | `latest.json` not attached to the release; repo went private; or the version in the manifest is not higher than the client's. |
| Clients see it but installation fails | Signature mismatch: the artifact was signed with a different key than the public key compiled into that client. |
| `Failed to add bundler type to the binary` during build | The binary was not relinked. See [BUILD_NOTES.md](BUILD_NOTES.md); the package may refuse to update. |
