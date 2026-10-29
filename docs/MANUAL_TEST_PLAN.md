# Manual test plan

What CI cannot check: the things a person has to look at or click. Run it
before a release, on each platform you ship. Each step says what to do and
what you should see; anything else is a bug worth filing.

Use a small real project (any git repository you know), not the Kara repo
itself, and start from a clean slate where a step says so.

## 1. Installers

| # | Do | Expect |
|---|---|---|
| 1.1 | macOS: open the `.dmg` from the *Desktop installers* workflow run, drag Kara to Applications | A normal window with Kara and an Applications shortcut |
| 1.2 | Open Kara from Applications | "Kara cannot be opened because the developer cannot be verified", not "Kara is damaged". Right-click → Open (or System Settings → Privacy & Security → Open Anyway) then opens it |
| 1.3 | Windows: run `Kara_*_x64-setup.exe` | SmartScreen → More info → Run anyway; installs without admin; Kara is in the Start menu |
| 1.4 | Windows: run the `.msi` instead, on a machine without the `.exe` install | Installs; Kara starts from the Start menu |
| 1.5 | Linux: `sudo apt install ./Kara_*_amd64.deb`, then run `kara-desktop`; separately `chmod +x` and run the `.AppImage` | Both open the window |
| 1.6 | On a machine that has never had the `kara` CLI installed, open a project in the app | A session starts (the app uses the copy it ships); no "kara not found" |
| 1.7 | Uninstall (drag to Trash / Apps & features / `apt remove kara`) | Removed cleanly; your projects and `~/…/kara` data are untouched |

## 2. Desktop app: first run and projects

| # | Do | Expect |
|---|---|---|
| 2.1 | Launch with no project chosen before | Empty state with "Open a project…"; CPU stays near idle (nothing is indexed until you choose) |
| 2.2 | Open a project (Cmd/Ctrl+O) | Project name in the toolbar; status bar shows the model and "ready" |
| 2.3 | Quit and relaunch | Same project reopens; window size and position are kept |
| 2.4 | Project tab → Recent: switch to another project and back | Each switch clears the old project's tabs, tree and search results |
| 2.4a | Drag a folder from Finder/Explorer onto the window; then drag a single file | Dashed outline while dragging; the folder opens as the project; the file is ignored with a hint |
| 2.5 | No model configured and no local model | Clear guidance on how to get one; nothing downloads without asking |
| 2.6 | Model pill → pick a local model that is not installed | Size, memory and license shown; download only after you confirm; progress bar; checksum verified |
| 2.7 | Pick a model too large for the machine | Refused with the memory numbers, not loaded |
| 2.8 | Settings → Where inference runs → A model server: Ollama (running), a model name, Save | Session restarts; status bar shows the Ollama model; `kara privacy` agrees |
| 2.9 | Same with Other OpenAI-compatible server and an empty address | "This server needs its address", nothing saved |
| 2.10 | Another Kara machine: address and token from `kara serve --inference` on a second machine; then a wrong token | Connects and restarts; the wrong token is refused with the server's message |
| 2.11 | This machine → Use this machine | Back to local models |

## 3. Desktop app: chat and changes

| # | Do | Expect |
|---|---|---|
| 3.1 | Ask "explain what this project does" | Streamed answer; blue activity lines ("Flying in…") while it reads files |
| 3.2 | Ask for a small change (e.g. add a function and a test) | File-change cards with +/− counts; Files tab shows M/+ badges and a count |
| 3.3 | Open diff on a change card | Side-by-side diff tab |
| 3.4 | Reject one file's change | That file goes back to its previous content; others stay |
| 3.5 | Toolbar undo | Kara's last batch is reverted; your own edits made meanwhile are kept |
| 3.6 | Plan mode: ask for a change | Plan only, no files touched; Approve plan button carries it out |
| 3.7 | Review mode | Risk review of the current diff; no files touched |
| 3.8 | Esc while Kara is working | The turn stops |
| 3.9 | Permission pill: Ask every time, then ask for something that runs a command | A prompt asks first; denying it stops that step |
| 3.10 | Click the context pill | Conversation is compacted; usage drops |
| 3.11 | New session (Cmd/Ctrl+N), then resume the old one from Sessions | History is back |
| 3.12 | Settings → diagnostics and privacy | Hardware, memory and where data goes; matches `kara doctor` / `kara privacy` |

## 4. Desktop app: editor, terminal, search, palette

| # | Do | Expect |
|---|---|---|
| 4.1 | Files tab: expand folders, open a file | Syntax-highlighted tab; `.gitignore`d folders (e.g. `node_modules`) are absent |
| 4.1a | Right-click a folder → New file…, type a name, Enter; then New folder…; then Rename… on the new file (Esc once to cancel, then for real) | Each appears in the tree; the new file opens; Esc leaves everything unchanged; a name that exists is refused |
| 4.1b | Rename a file that's open with unsaved edits; save, then rename it | First refused ("save or close"), then the tab follows the new name |
| 4.1c | Right-click a file → Move to Trash / Recycle Bin, confirm; then a folder with an open, edited file inside | Gone from the tree and recoverable from the Trash/Recycle Bin; the second warns about unsaved changes and closes the tab |
| 4.2 | Type in the file | A dot replaces the tab's ✕ |
| 4.3 | Cmd/Ctrl+S | Saved to disk (check with another editor); dot goes away |
| 4.4 | Edit a file, switch tabs, come back | Unsaved text and undo history are still there |
| 4.5 | Open a file, change it in another editor, then edit and save in Kara | Save refused: "changed on disk since it was opened"; the other edit is kept |
| 4.5a | Open a file, ask Kara to change it; also edit it in another editor and switch back to Kara | The open tab updates to the new content each time, keeping its scroll position |
| 4.5b | Same, but with your own unsaved edits in the tab | Your edits stay; a warning says it changed on disk; saving is refused (4.5) |
| 4.6 | Close a tab with unsaved changes (Cmd/Ctrl+W) | Asks before discarding |
| 4.6a | With an unsaved tab: close the window, press Cmd+Q (macOS), and open another project | Each asks first; Keep editing leaves everything as it was |
| 4.7 | Cmd/Ctrl+J | Terminal panel opens in the project folder; `ls`, colors, arrow-key history and Ctrl+C work; resizing the window reflows it |
| 4.8 | `exit` in the terminal, close and reopen the panel | A fresh shell |
| 4.9 | Shift+Cmd/Ctrl+F, search a word | Results grouped by file; clicking one opens the file at that line, selected |
| 4.10 | Toggle Aa (match case) and .* (regex), then type an invalid regex like `(` | Results update; invalid regex shows an error, not a crash |
| 4.11 | Cmd/Ctrl+P, type part of a file name (e.g. `mts` for `main.ts`) | Best match first, matched letters highlighted; Enter opens it |
| 4.12 | Shift+Cmd/Ctrl+P, or type `>` in quick open | Commands list; "Mode: Plan" switches the mode; a slash command fills the chat box |
| 4.12a | Status bar in a git repo: switch branch in a terminal and come back; let Kara change a file; check out a commit (detached) | "⎇ branch · N changed" updates on focus and after the turn; detached shows "detached at <sha>"; hidden outside a repo |
| 4.13 | Toggle light/dark theme (toolbar) | Whole app, editor and diffs switch; red/blue stay readable in both |

## 5. Terminal CLI

| # | Do | Expect |
|---|---|---|
| 5.1 | `kara --version`, `kara doctor`, `kara privacy`, `kara models list` | Output without errors; the model list matches what the desktop app shows |
| 5.2 | `kara` in a project | Full-screen UI, red header and input border, blue activity lines |
| 5.3 | `/help`, `/matrix`, `/diff`, `/undo`, `/exit` | Each works; `/exit` restores the terminal |
| 5.4 | A task that edits and tests | Same behaviour as 3.2, in the terminal |

## 6. VS Code extension

| # | Do | Expect |
|---|---|---|
| 6.1 | Install the `.vsix` from the CI artifact (Extensions → … → Install from VSIX) | Kara diamond in the activity bar |
| 6.2 | Open the Kara view in a project | Connects; model and permission mode shown |
| 6.3 | Ask for a change | Streaming, tool activity, diff preview with accept/reject |
| 6.4 | Select code and ask about it | The answer refers to the selection |
| 6.5 | With a red squiggle in the file, ask Kara to fix it | Diagnostics are used |

## 7. Weak hardware

On the lowest machine you have (e.g. 8 GB RAM, 4 cores):

| # | Do | Expect |
|---|---|---|
| 7.1 | `kara doctor` | Recommends a model that fits, or a remote option if none does |
| 7.2 | Run a task with the recommended model while using the machine | It stays usable: Kara leaves CPU cores free and refuses models that would exhaust memory |
