# Privacy

The keyboard is offline. It has no network code, asks for no network permission,
and collects and sends nothing.

## Guarantees

* **No `INTERNET` permission** is declared in any manifest
  (`java/AndroidManifest.xml`, flavor manifests, test manifests), so Android
  refuses any socket the app tries to open. `ACCESS_NETWORK_STATE` is explicitly
  removed as well.
* **No telemetry, analytics, crash reporting or advertising SDK sends data.**
  ACRA is no longer initialised, so no crash report is ever built, stored for
  upload or mailed; the `Copy logs` action in the settings still assembles a
  report on the device, only when the user taps it, and puts it on the
  clipboard.
* **No update checking.** `checkForUpdate()` is a documented no-op; the update
  job is never scheduled (`UPDATE_CHECKING_NETWORK` is false and the scheduler
  returns immediately).
* **No dictionary downloads.** The AOSP dictionary provider (`dictionarypack/`)
  has been removed; only its `MD5Calculator` helper survives because the theme
  importer uses it to verify local files.
* **No `java.net` networking, no OkHttp.** The only remaining `java.net` uses are
  `URLEncoder`/`URLDecoder`, which are pure string helpers. The dead
  `latin/network/` HTTP client and its tests were deleted, and the APK-download
  screen was deleted.

## What still touches the outside world (by design)

* `openURI(...)` opens a link in the user's browser — an explicit tap on a
  settings entry, never automatic.
* Voice-input / translation models are downloaded only when the user opens the
  model list and asks for one. Those downloads are performed by the system
  downloader/browser, not by the keyboard's process.
* Importing dictionaries or themes reads a file the user picked.

## Verifying the claim

```sh
# 1. Nothing declares the INTERNET permission
grep -rn "permission.INTERNET" --include=*.xml .

# 2. No networking APIs are referenced from app code
grep -rn "HttpURLConnection\|okhttp3\|java\.net\.Socket" --include=*.java --include=*.kt \
     java/ common/ tests/

# 3. The built APK requests no network permission
aapt dump permissions app/build/outputs/apk/*/release/*.apk | grep -i internet   # prints nothing
```

## Rust core

`rust/` (the new dictionary/suggestion engine) links only the Rust standard
library. It contains no socket code, no `std::net` import, no HTTP client and no
dependencies at all, so it cannot phone home even if it wanted to.
