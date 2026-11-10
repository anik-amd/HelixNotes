# Flatpak

This builds HelixNotes from the current checkout inside the GNOME 51 SDK. Rust
and Node dependencies are pinned by the existing lockfiles and downloaded before
the sandboxed, offline build. The shared-modules submodule supplies the Ayatana
tray libraries.

## Build

Run from the repository root:

```sh
git submodule update --init packaging/flatpak/shared-modules
flatpak install --user flathub org.flatpak.Builder org.gnome.Platform//51 org.gnome.Sdk//51 org.freedesktop.Sdk.Extension.node24//26.08 org.freedesktop.Sdk.Extension.rust-stable//26.08
flatpak run org.flatpak.Builder --user --force-clean --state-dir=src-tauri/target/flatpak/state --repo=src-tauri/target/flatpak/repo src-tauri/target/flatpak/build packaging/flatpak/com.helixnotes.app.yml
flatpak build-bundle src-tauri/target/flatpak/repo src-tauri/target/flatpak/HelixNotes_1.3.7_x86_64.flatpak com.helixnotes.app stable --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
```

The commands require a user-local Flathub remote. If only a system remote exists,
add the user remote first:

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
```

## Install and run

```sh
flatpak install --user src-tauri/target/flatpak/HelixNotes_1.3.7_x86_64.flatpak
flatpak run com.helixnotes.app
```

Choose vault folders using the folder picker. Flatpak grants access to the chosen
folders through its document portal; the package does not request access to the
whole home directory. Its settings are separate from native installations.
The document portal does not forward external file-change notifications, so this
build checks for changes every two seconds using the existing polling watcher.
Metadata polling can miss edits within the same timestamp second or edits that
preserve an earlier modification time. Reopen the vault to refresh those changes.

The build sets `HELIXNOTES_INSTALL_TYPE=flatpak`, which suppresses the in-app
updater. Install a newer bundle to update this local package. Automatic Flatpak
updates require a hosted repository or a Flathub release; neither is published by
these build commands. The bundle references Flathub for its runtime, which is
installed separately.

Close native HelixNotes before testing: native and Flatpak installations use the
same single-instance application identifier.

## Refresh dependencies

Regenerate both source lists whenever their lockfiles change. The initial lists
were generated with flatpak-builder-tools commit
`9d86d336a39b9af38b86f114b6ce5820fe260f91`:

```sh
uv run --project /path/to/flatpak-builder-tools/node python -m flatpak_node_generator --no-requests-cache --pnpm-store-version v11 --node-sdk-extension org.freedesktop.Sdk.Extension.node24//26.08 -o packaging/flatpak/node-sources.json pnpm pnpm-lock.yaml
uv run /path/to/flatpak-builder-tools/cargo/flatpak-cargo-generator.py -o packaging/flatpak/cargo-sources.json src-tauri/Cargo.lock
```

Keep the pinned pnpm archive/version in the manifest in step with
`package.json`'s `packageManager`. pnpm 11 uses the v11 store; the generator's
default v10 store does not work with it.

The offline install uses `--trust-lockfile` for this reviewed, pinned lockfile.
Flatpak verifies the downloaded sources against the generated checksums. This
keeps pnpm from querying registry metadata and attestations during the offline
build; it does not grant network access to the build sandbox.

A Flathub submission also needs a pinned Git source in place of the local
directory source, release metadata and a content-rating assessment.
