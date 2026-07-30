# Fonts

**Archivo**, the Brume UI typeface named in `../css/tokens.css`.

These are the variable weight axis (300–700), subset by Google Fonts:

| File | Subset |
|---|---|
| `archivo-latin-var.woff2` | latin |
| `archivo-latin-ext-var.woff2` | latin-ext |

## Why these are committed

The brand kit deliberately ships no font files — its wordmark travels as baked outlines in
`../../tools/wordmark.json` so the kit can rebuild with no fonts and no network access. That
works for the logo, but not for UI text.

Archivo is therefore vendored here and loaded with `@font-face`. Fetching it from Google Fonts
at runtime is not an option: the installer and the browser chrome both need to render before
any network request would complete, and an installer that reaches out to a CDN to draw its own
UI is both slow and a privacy leak.

## Licence

Archivo is licensed under the **SIL Open Font License 1.1**, which permits bundling and
redistribution. Designed by Omnibus-Type.

- <https://fonts.google.com/specimen/Archivo/about>
- <https://openfontlicense.org/>

## Updating

Fetch the current URLs from the Google Fonts CSS API and re-download:

```bash
curl -H "User-Agent: Mozilla/5.0 ... Chrome/120" \
  "https://fonts.googleapis.com/css2?family=Archivo:wght@300..700&display=swap"
```

A modern browser User-Agent is required, or the API returns legacy TTF rather than woff2.
