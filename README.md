# niruprodrive

Proton Drive for [NIRUX](https://github.com/nirucon/nirux): a backup of the
folders you choose on each computer, files sent, fetched and shared, a cloud
in the bar and a small window in the NIRU theme. One Rust program of a few
megabytes that sits idle at about 6 MB of memory and no CPU.

> niruprodrive is a third-party application. It is not made or supported by
> Proton. It works through Proton's own
> [Proton Drive CLI](https://proton.me/support/drive-cli), which does the
> sign-in, the encryption and the talking to Proton.

## What it does

- **Backup, per computer.** Each computer chooses its own folders on the
  Backup tab (drop them from NIRUFM, or press `A`). They are backed up to
  `My files/NIRUX/<computer>/` hourly, daily or only when you ask, and wait
  for mains power on a laptop unless you say otherwise.
- **Nothing is ever removed in the Drive.** A file deleted here stays there;
  a changed file becomes a new revision and Proton Drive keeps the old ones.
  Only what changed is sent: a new folder goes in one upload, changed files
  in batches, and the Drive is never walked from here.
- **Send, fetch and share.** Drop files on the window to send them to the
  folder shown; `D` downloads to `~/Downloads`; `C` copies a public link,
  `Shift+C` one that ends after 7 days, `X` removes a link. Files sent from
  NIRUFM or the command line go to `My files/NIRUX/Uploads`.
- **The bar.** A pixel cloud in NIRUBAR's tray: a tick when all is sent, an
  arrow while sending, bars when paused, `!` after a failure, an outline when
  signed out. Click for the window, right-click for a menu.
- **NIRUFM.** While niruprodrive runs, NIRUFM's menu has *Send to Proton
  Drive* and *Copy Proton Drive link*, and files in backed-up folders carry a
  small badge: sent, or waiting for the next backup. On a computer without
  niruprodrive, nothing about it shows.
- **Retro, in the theme.** Pixel letters, a dashed drop zone, the theme's
  colours (live when the theme changes).

## Safety

niruprodrive can only ask the CLI for what is listed in `src/cli.rs`: list,
look up, make a folder, upload, download, and add or remove a public link.
There is no command in it that trashes, deletes, moves, renames or replaces
anything. New revisions and new folders are only made inside
`My files/NIRUX`; elsewhere an upload with a name that exists gets a new name.
Downloads never overwrite a local file. The tests check every command line
niruprodrive can make, and the end-to-end test runs against a stand-in CLI
that fails loudly if anything else is ever asked.

Sign-in happens on Proton's page in your browser, with two-factor if you use
it; niruprodrive never sees your password. The session is kept by the CLI in
the keyring (the Secret Service), or in `pass` with
`credentials = "pass"`.

## Install

With NIRUX, on the computers you want it on:

```
nirux extras install niruprodrive
```

Or from a checkout: `./install.sh` downloads the release binary (checked
with SHA-256) and Proton's CLI, pinned to a tested version and checked with
the SHA-512 Proton publishes, into `~/.local/lib/niruprodrive/`. It adds a
menu entry and autostart and starts the service. `./install.sh --build`
builds with cargo instead. `./uninstall.sh` signs out and removes it
(`--purge` also the settings); your files in the Drive are untouched.

## Use

```
niruprodrive                  the window (starts the service when needed)
niruprodrive service          the tray icon and the backups, no window
niruprodrive upload PATH...   send files or folders to My files/NIRUX/Uploads
niruprodrive link PATH [DAYS] copy a public link (DAYS makes it expire)
niruprodrive backup           back up now
niruprodrive status           what it is doing, as JSON
niruprodrive quit
```

Keys in the window: `1` `2` `3` (or `Tab`) for Drive, Transfers and Backup;
`Enter` opens, `Backspace` goes up, `F5` refreshes; on the Backup tab `B`
backs up now, `P` pauses, `S` changes the schedule, `O` the battery rule and
`E` opens the settings file. `Esc` stops what runs or closes the window,
`Ctrl+Q` quits.

## Settings

`~/.config/niruprodrive/config.toml`, written by the window too:

```toml
[backup]
folders = ["~/Documents", "~/Pictures"]
exclude = [".cache", "node_modules", "target", "*.tmp", "*.iso"]
every = "hourly"        # manual, hourly or daily
on_battery = false
max_file_mb = 4096

[drive]
folder = "NIRUX"        # My files/NIRUX
computer = "othala"     # this computer's folder in it (the host name)
downloads = "~/Downloads"
credentials = "keychain" # or "pass"
```

What was sent is recorded in `~/.local/state/niruprodrive/backup.json`. To
restore, open This computer on the Drive tab and download (`D`).

## Build and test

```
cargo test
cargo build --release
sh tests/xvfb.sh target/release/niruprodrive    # Xvfb, xdotool, xclip
```

The end-to-end test signs in, backs up, changes and deletes files, sends,
links and checks the badges, all against `tests/fake-proton-drive`.

MIT licence.
