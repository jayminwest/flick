# Install and update

## From source

Flick needs macOS 13 or later and a Rust toolchain.

```bash
git clone https://github.com/jayminwest/flick
cd flick
./scripts/bundle.sh --install
```

The script builds `~/Applications/Flick.app` and starts it. Press `⌥⇧Space` to open the launcher.

Window management, the window switcher, and auto-paste need Accessibility permission. macOS asks the first time you use one of them. Turn Flick on in **System Settings → Privacy & Security → Accessibility**.

> [!NOTE]
> The script signs Flick with your Apple Development certificate when you have one. With that identity, macOS keeps the Accessibility permission across rebuilds. Without one, the script signs ad hoc, and macOS asks again after each rebuild.

## Prebuilt app

Each [release](https://github.com/jayminwest/flick/releases) has an Apple Silicon build. It is not notarized, so remove the quarantine flag after you unzip it:

```bash
xattr -dr com.apple.quarantine Flick.app
mv Flick.app ~/Applications/
```

## Launch at login

Add Flick to **System Settings → General → Login Items**. With nix-darwin and home-manager, use a launchd agent:

```nix
launchd.agents.flick = {
  enable = true;
  config = {
    ProgramArguments = [ "/Users/you/Applications/Flick.app/Contents/MacOS/Flick" ];
    RunAtLoad = true;
    KeepAlive.SuccessfulExit = false; # restart on crash; "Quit Flick" stays quit
    ProcessType = "Interactive";
  };
};
```

`bundle.sh --install` and the in-app rebuild install through `scripts/relaunch.sh`. It copies the new bundle next to the old one, checks its signature, and swaps the two with renames, so a failed install leaves the old app in place. Then it stops Flick, waits until the old process exits, and starts Flick through this agent when it exists (`launchctl kickstart -k`), else with `open`.

## Rebuild from the checkout

Flick can rebuild itself from the local clone it was built from. It does not fetch, pull or push: it builds the commits that are already in the checkout.

- **Flick Version** shows the installed commit and build time, and how many commits the checkout is ahead, with their subjects. Flick checks the checkout with local git commands when the launcher opens, at most once per 30 s.
- **Rebuild Available** shows when the checkout's `HEAD` is not the installed commit.
- **Rebuild Flick** exports `HEAD` with `git archive` and builds that, so uncommitted edits stay out. **Rebuild Flick (Dirty)** builds the tree as it is, and the version ends in `-dirty`.

A rebuild runs in the background. The build view shows the elapsed time and the last log line, with **Cancel Build** and **Open Build Log**. When the build succeeds, Flick installs the new app and restarts. When it fails, Flick opens the log, and the installed app and the running process do not change. Flick never rebuilds by itself.

```bash
flick flick version                    # installed sha, build time, clean or dirty; the checkout's HEAD
flick flick rebuild                    # build HEAD; prints the log path and returns at once
flick flick rebuild --dirty            # build the tree as it is
flick flick rebuild --ref my-branch    # build another local rev
flick flick status                     # idle, building <n>s, installing, installed, failed or cancelled
flick flick cancel                     # stop the build that runs
```

Details:

- The build runs `scripts/bundle.sh` with `cargo build --release --locked --offline` through your login shell (`$SHELL -lc`), so `cargo` must be on your login-shell `PATH`. If it is not, the build fails with "cargo not found on login-shell PATH".
- The build is offline. If the checkout needs a crate that cargo has not downloaded, the build fails with a hint: run `cargo fetch` in the checkout, then rebuild.
- Compiled output goes to `<checkout>/target/flick-rebuild`, apart from `target/`, so a rebuild does not wait on other cargo commands. It takes about 1 GB; `cargo clean --target-dir target/flick-rebuild` removes it. The exported tree is in `~/Library/Caches/Flick/rebuild`.
- The log is `~/Library/Logs/Flick/rebuild.log`. The log of the build before it is `rebuild.log.1`.
- `--source <dir>` builds another checkout or worktree, for example an agent's worktree, to try it before you merge it. The installed app then records that directory as its checkout, so **Flick Version** and **Rebuild Available** compare against it until you rebuild from the main checkout (flick-be14).
- Without an Apple Development identity the bundle is signed ad hoc, and macOS asks for Accessibility again after each rebuild.

## Rebuild settings

```toml
[flick]
source = "~/Projects/flick"   # the checkout to compare and rebuild; default: the one this app was built from
check_on_open = true          # check the checkout when the launcher opens (at most once per 30 s)
gates = false                 # run scripts/check-all.sh --bail before each rebuild (takes minutes)
```
