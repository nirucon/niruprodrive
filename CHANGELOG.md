# niruprodrive 0.1.6

- A tray click or a key is never left waiting. Events that arrived while
  the window was painted sat in the X connection's queue, and the service
  slept until the next event or a minute had passed: about one tray click or
  Escape in ten did nothing at first.

# niruprodrive 0.1.5

- Closing the window tells the window manager it is withdrawn (ICCCM), also
  when the window manager had hidden it on another workspace. Before, the
  window could come back empty (black) on that workspace, taking half of it.

# niruprodrive 0.1.4

- The service stays up. A small supervisor starts it again when it is
  killed or stopped by a signal sent to it alone (on niruxwork it was
  stopped half a minute after a resume, and the cloud was gone until the
  next login). Quit, `niruprodrive quit`, a logout and a stop of both
  processes still end it; one that keeps failing is given up on.
- What stopped it is written to `~/.local/state/niruprodrive/service.log`:
  the signal and the process that sent it.
- A closed X display (a logout) ends the service, so the next login starts
  a new one with its icon instead of finding an old one without a bar.
- A reader that goes away no longer ends the service (SIGPIPE).
- The installer starts the service under the user's systemd manager when
  there is one, not inside the SSH login of a rollout.

# niruprodrive 0.1.3

- The cloud in the bar is painted whenever the bar shows it: a first paint
  before the tray had mapped it was lost and the icon stayed black.
- Signed in but no folder chosen yet: the cloud shows a dash, not a tick,
  and its name says that no folders are chosen.

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
