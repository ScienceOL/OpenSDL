# CLI releases

OpenSDL uses cargo-dist 0.32.0, configured in `dist-workspace.toml`.
The Rust package is `lab-cli`; its executable is `lab`. Consequently the
installers are `lab-cli-installer.sh` and `lab-cli-installer.ps1`, and the
installed updater is `lab-cli-update`.

## Prepare and verify

1. Change `workspace.package.version` in `Cargo.toml` to the new version and
   run `cargo check --workspace` to update the workspace entries in `Cargo.lock`.
2. Update the README when commands, supported targets, or prerequisites change.
3. Run `cargo fmt --all --check` and `cargo test --workspace --locked`.
4. If dist configuration changed, run `dist generate`; do not hand-edit the
   generated `.github/workflows/release.yml`.
5. Run `dist plan --output-format=json`. Check that its release version,
   installers, executable, and six platform archives match the README.
6. Open a pull request. The release workflow runs workspace tests on three
   operating systems and builds all release targets, archives, and installers.
   Merge only after these checks pass.

## Publish

From the merged, clean main branch, create and push the tag matching the
workspace version. For example, for v0.2.0:

```bash
git tag -a v0.2.0 -m "OpenSDL v0.2.0"
git push origin v0.2.0
```

The tag workflow reruns the checks, builds the release artifacts, and creates
the GitHub Release. Do not replace an existing tag or overwrite artifacts.

## Verify the public installation

Check both README `releases/latest/download/` installer URLs after publication.
Install from the public URL into an isolated directory to avoid replacing a
developer's existing CLI. For example, in a POSIX shell:

```bash
install_root="$(mktemp -d)"
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ScienceOL/OpenSDL/releases/latest/download/lab-cli-installer.sh \
  | LAB_CLI_INSTALL_DIR="$install_root" LAB_CLI_NO_MODIFY_PATH=1 sh
"$install_root/bin/lab" --version
"$install_root/bin/lab" --help
"$install_root/bin/lab-cli-update" --help
```

On Windows, use a temporary `LAB_CLI_INSTALL_DIR` and set
`LAB_CLI_NO_MODIFY_PATH=1` before running the README's PowerShell installer.
Check `lab.exe` and `lab-cli-update.exe` in its `bin` subdirectory.

In an isolated directory, write `smoke.yaml` containing `{}` to disable MQTT
and hardware discovery. Start the installed CLI with:

```bash
lab serve --instance install-smoke --config smoke.yaml --data-dir state --socket disabled --listen 127.0.0.1:0
```

From another terminal using the same installed binary, run
`lab --instance install-smoke status`, `lab --instance install-smoke device list`,
and `lab --instance install-smoke stop`. Confirm the server exits, reports the
released version, and has no devices. This checks the installed server/client
path without opening hardware ports. Verify on Linux, macOS, and Windows;
record any platforms not exercised in the delivery notes.
