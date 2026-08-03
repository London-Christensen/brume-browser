# The update feed

`latest.json` in this directory is what every installed copy of Brume checks to
find out whether a newer version exists. It is served straight from the
repository:

```
https://raw.githubusercontent.com/London-Christensen/brume-browser/main/updates/latest.json
```

**Do not edit it by hand.** `tools/new-release.ps1` writes it as part of cutting
a release, and the `signature` field has to match the exact bytes of the
installer the `url` points at. A hand-edited mismatch is rejected by every client
silently.

## Why it lives here and not on the release page

It used to be a release asset. That meant the release page listed several files
when only one of them, `Brume-Setup.exe`, is something a person should ever
download, and there was nothing to say which. Serving the feed from the
repository leaves the release page with the installer and the two source
archives GitHub attaches on its own.

The installer the feed points at is `Brume-Setup.exe`, the same file a person
downloads. It carries the NSIS installer inside it and hands over to it when run
with `/UPDATE`, so there is no second installer to publish either.

## This URL can never change

It is compiled into every copy of Brume that ships. An install that is already
out there will keep asking this exact address forever, so moving or renaming the
file strands it with no way to recover except a manual reinstall. Treat the path
as permanent.
