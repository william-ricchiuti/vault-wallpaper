// Vault Wallpaper companion: tells the wallpaper app which note you have open.
//
// It writes one small JSON file inside the vault,
//   <vault>/.obsidian/plugins/vault-wallpaper/state.json
// which the app watches. Nothing leaves the machine: no port, no network.
//
// The file is written atomically (temp file + rename), debounced, and only when its
// content changes, plus a heartbeat every couple of minutes so the app can tell
// Obsidian is still open (it ignores a state older than five minutes).

import { App, FileSystemAdapter, Plugin, PluginSettingTab, Setting, TAbstractFile, TFile } from "obsidian";
import { promises as fs } from "fs";
import * as path from "path";

/// Bumped when the file's meaning changes; the app ignores versions it doesn't know.
const VERSION = 1;
/// Coalesces a burst of events (switching notes fires several) into one write.
const DEBOUNCE_MS = 250;
/// Refreshes `updated` so the app knows the state is live. Must stay well under the
/// app's staleness limit (5 minutes).
const HEARTBEAT_MS = 2 * 60 * 1000;

interface Settings {
  enabled: boolean;
}

/// What the app reads. Paths are vault-relative with "/" separators, as Obsidian
/// reports them ("Folder/Note.md").
interface State {
  version: number;
  /// The note in the active editor, or null when none (or the plugin is off).
  active: string | null;
  /// Obsidian's own resolved outgoing links of the active note (from
  /// metadataCache.resolvedLinks), as vault-relative paths.
  links: string[];
  /// The note saved most recently while open in Obsidian, and when (ms since epoch).
  saved: string | null;
  savedAt: number | null;
  /// When this state was written (ms since epoch).
  updated: number;
}

export default class VaultWallpaperPlugin extends Plugin {
  settings: Settings = { enabled: true };
  private saved: string | null = null;
  private savedAt: number | null = null;
  /// The last content written, without `updated`, to skip writes that change nothing.
  private lastKey = "";
  private lastWrite = 0;
  private timer: number | null = null;
  private writing: Promise<void> = Promise.resolve();

  async onload() {
    this.settings = Object.assign({ enabled: true }, await this.loadData());
    this.addSettingTab(new VaultWallpaperSettingTab(this.app, this));

    this.registerEvent(this.app.workspace.on("file-open", () => this.schedule()));
    this.registerEvent(this.app.workspace.on("active-leaf-change", () => this.schedule()));
    this.registerEvent(this.app.vault.on("modify", (f) => this.onModify(f)));
    this.registerEvent(this.app.vault.on("rename", () => this.schedule()));
    // Links of the active note change as you type them.
    this.registerEvent(
      this.app.metadataCache.on("resolve", (f) => {
        if (f.path === this.app.workspace.getActiveFile()?.path) this.schedule();
      }),
    );
    this.registerInterval(window.setInterval(() => this.heartbeat(), HEARTBEAT_MS));
    this.app.workspace.onLayoutReady(() => this.schedule());
  }

  async onunload() {
    if (this.timer !== null) window.clearTimeout(this.timer);
    // Tell the app right away that nothing is open, instead of waiting for staleness.
    await this.write(this.state(true), true);
  }

  async setEnabled(on: boolean) {
    this.settings.enabled = on;
    await this.saveData(this.settings);
    this.schedule();
  }

  /// A save counts only for notes open in Obsidian, so a sync client landing files in
  /// the background doesn't look like you editing.
  private onModify(f: TAbstractFile) {
    if (!(f instanceof TFile) || f.extension !== "md" || !this.isOpen(f)) return;
    this.saved = f.path;
    this.savedAt = Date.now();
    this.schedule();
  }

  private isOpen(f: TFile): boolean {
    let open = false;
    this.app.workspace.iterateAllLeaves((leaf) => {
      const file = (leaf.view as { file?: TFile | null }).file;
      if (file && file.path === f.path) open = true;
    });
    return open;
  }

  private heartbeat() {
    if (Date.now() - this.lastWrite >= HEARTBEAT_MS - 1000) this.schedule(true);
  }

  private schedule(force = false) {
    if (this.timer !== null) window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      this.timer = null;
      void this.write(this.state(false), force);
    }, DEBOUNCE_MS);
  }

  private state(closing: boolean): State {
    const on = this.settings.enabled && !closing;
    const file = on ? this.app.workspace.getActiveFile() : null;
    const active = file && file.extension === "md" ? file.path : null;
    const resolved = active ? this.app.metadataCache.resolvedLinks[active] : undefined;
    return {
      version: VERSION,
      active,
      links: resolved ? Object.keys(resolved).filter((p) => p !== active).sort() : [],
      saved: on ? this.saved : null,
      savedAt: on ? this.savedAt : null,
      updated: Date.now(),
    };
  }

  /// Writes atomically: a temp file in the same folder, then a rename over the old one,
  /// so the app never reads half a file. Writes are serialised.
  private write(state: State, force: boolean): Promise<void> {
    const { updated: _, ...rest } = state;
    const key = JSON.stringify(rest);
    if (!force && key === this.lastKey) return this.writing;
    this.lastKey = key;
    this.lastWrite = state.updated;
    const adapter = this.app.vault.adapter;
    if (!(adapter instanceof FileSystemAdapter) || !this.manifest.dir) return this.writing;
    const dir = path.join(adapter.getBasePath(), this.manifest.dir);
    const target = path.join(dir, "state.json");
    const tmp = `${target}.tmp`;
    const body = JSON.stringify(state, null, 2) + "\n";
    this.writing = this.writing
      .then(async () => {
        await fs.mkdir(dir, { recursive: true });
        await fs.writeFile(tmp, body, "utf8");
        await fs.rename(tmp, target);
      })
      .catch((e) => console.error("vault-wallpaper: could not write state.json", e));
    return this.writing;
  }
}

class VaultWallpaperSettingTab extends PluginSettingTab {
  constructor(
    app: App,
    private plugin: VaultWallpaperPlugin,
  ) {
    super(app, plugin);
  }

  display(): void {
    this.containerEl.empty();
    new Setting(this.containerEl)
      .setName("Share the open note with the wallpaper")
      .setDesc("Writes the open note's path to state.json in this plugin's folder. Nothing leaves your computer.")
      .addToggle((t) => t.setValue(this.plugin.settings.enabled).onChange((v) => this.plugin.setEnabled(v)));
  }
}
