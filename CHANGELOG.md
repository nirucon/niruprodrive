# niruprodrive 0.1.2

- The window opens on the workspace you are on: a click on the cloud (or
  `niruprodrive`) brings a hidden, minimised or elsewhere-open window to
  this workspace, focused. The cloud hides it only when it is in view here.

# niruprodrive 0.1.1

- Sign-in works when the service was started by the installer: the service
  and the Proton Drive CLI now run in the home folder, not in the
  installer's temporary folder (which is deleted, and the CLI refused to
  run there: "no contact").
- The cloud in the bar is easier to see before sign-in: a full cloud in a
  light grey with a dash, instead of a faint outline.

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
