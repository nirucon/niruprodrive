# niruprodrive 0.1.0

The first version.

- Backup of chosen folders per computer to `My files/NIRUX/<computer>/`:
  hourly, daily or on request, waiting for mains power on a laptop. Only
  changes are sent; nothing is ever removed in the Drive, and changed files
  keep their old revisions.
- Send files by dropping them on the window, from NIRUFM or with
  `niruprodrive upload`; download with `D`; public links (also ones that
  expire) with `C`, removed with `X`.
- A pixel cloud in NIRUBAR's tray (XEmbed) with the state, a menu on
  right-click, and a window with Drive, Transfers and Backup tabs in the
  NIRU theme.
- A socket for NIRUFM: its menu sends files and copies links, and badges show
  what is backed up.
- Works through Proton's Proton Drive CLI 0.9.0, pinned and checked with
  SHA-512 by the installer. Only listing, uploading, downloading, making
  folders and links can be asked of it.
