import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open, save } from "@tauri-apps/plugin-dialog";

type Versions = { spotify: string | null; spicetify: string | null; marketplace: string | null };
type Theme = { name: string | null; colourScheme: string | null; source: string };
type Summary = {
  id: string; name: string | null; createdAt: string; reason: string; source: string; versions: Versions; theme: Theme;
  extensions: number; keys: number; unassigned: number; bytes: number; importedAt: string | null; shared: boolean;
};
type Imported = { id: string; alreadyThere: boolean; extensions: number; theme: string | null; colourScheme: string | null; shared: boolean };
type KeyInfo = { key: string; size: number };
type ExtensionDetails = {
  id: string; name: string; kind: string; source: string | null; keys: string[]; databases: string[];
  globals: string[]; scanned: boolean; keyInfo: KeyInfo[]; files: [string, number, string][]; records: number;
};
type Details = {
  id: string; name: string | null; createdAt: string; reason: string; source: string; versions: Versions; theme: Theme;
  marketplace: { storage: string; keys: string[] }; extensions: ExtensionDetails[];
  spotifyKeys: KeyInfo[]; unassigned: KeyInfo[]; notes: string[];
};
type Status = {
  detection: { spotify: { version: string | null } | null; storeSpotify: boolean; spicetify: { exe: string | null } | null };
  spotifyState: "closed" | "connected" | "runningWithoutPort"; spicetifyVersion: string | null; profilesDir: string;
};
type Check = { warnings: string[]; blockers: string[] };
type Step = { name: string; ok: boolean; detail: string };
type Report = { ok: boolean; needsConfirmation: Check | null; backupId: string | null; steps: Step[] };
type SelfTest = { summary: string; clipboard: string; results: { id: number; name: string; status: string; detail: string }[] };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const short = (v: string | null) => (v ? v.split(".").filter((p) => /^\d+$/.test(p)).join(".") : "?");
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n: number) => String(n).padStart(2, "0");
/** Local time as `27 Nov 2026, 10:10:04`, the same clock time as the profile's file name. */
const when = (iso: string) => {
  const d = new Date(iso);
  return `${pad(d.getDate())} ${MONTHS[d.getMonth()]} ${d.getFullYear()}, ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
const size = (n: number) => (n < 1024 ? `${n} B` : n < 1 << 20 ? `${(n / 1024).toFixed(1)} KB` : `${(n / (1 << 20)).toFixed(1)} MB`);
const KIND: Record<string, string> = { local: "Local extension", marketplace: "Marketplace extension", theme: "Theme", "custom-app": "Custom app", snippet: "Snippet", spicetify: "Spicetify" };

let busy = false;

async function refreshStatus() {
  const el = $("status");
  try {
    const s = await invoke<Status>("status");
    const parts: string[] = [s.spotifyState === "connected" ? `<span class="connected">connected</span>` : `<span class="disconnected">not connected</span>`];
    const sp = s.detection.spotify;
    if (!sp) parts.push(s.detection.storeSpotify ? `<span class="bad">Microsoft Store Spotify isn't supported by Spicetify. Install the regular version from spotify.com.</span>` : `<span class="bad">Spotify isn't installed</span>`);
    else parts.push(`Spotify ${short(sp.version)}`);
    parts.push(s.spicetifyVersion ? `Spicetify ${esc(s.spicetifyVersion)}` : `<span class="bad">Spicetify isn't installed</span>`);
    el.innerHTML = parts.join(" · ");
    const open = s.spotifyState === "runningWithoutPort";
    $("banner").hidden = !open;
    if (!open) $("banner-error").hidden = true;
    $<HTMLButtonElement>("capture").disabled = open;
  } catch (e) {
    el.textContent = String(e);
  }
}

let closing = false;
async function closeSpotify() {
  if (closing || busy) return;
  closing = true;
  const btn = $<HTMLButtonElement>("close-spotify");
  const err = $("banner-error");
  btn.disabled = true;
  btn.textContent = "Closing…";
  err.hidden = true;
  try {
    await invoke("close_spotify");
  } catch (e) {
    err.textContent = String(e);
    err.hidden = false;
  } finally {
    closing = false;
    btn.disabled = false;
    btn.textContent = "Close Spotify for me";
    refreshStatus();
  }
}

async function refreshProfiles() {
  const list = await invoke<Summary[]>("list_profiles");
  const el = $("profiles");
  if (!list.length) {
    el.innerHTML = `<p class="empty">No profiles yet. Press Back up settings to make the first one.</p>`;
    return;
  }
  el.innerHTML = list
    .map(
      (p) => `<article class="profile">
        <div class="info">
          <h3><span class="title">${p.name ? esc(p.name) : when(p.createdAt)}</span><button class="rename" data-rename="${esc(p.id)}" data-name="${esc(p.name ?? "")}" data-placeholder="${esc(when(p.createdAt))}" title="Rename" aria-label="Rename">${PENCIL}</button>${p.reason === "before-restore" ? `<span class="badge">automatic, before a restore</span>` : ""}${p.source === "leveldb" ? `<span class="badge warn">read with Spotify closed</span>` : ""}${p.importedAt ? `<span class="badge">imported ${when(p.importedAt)}</span>` : ""}${p.shared ? `<span class="badge">shared copy</span>` : ""}</h3>
          ${p.name ? `<p class="muted">${when(p.createdAt)}</p>` : ""}
          <p class="muted">${p.extensions} extensions · theme: ${p.theme.name ? `${esc(p.theme.name)}${p.theme.colourScheme ? ` (${esc(p.theme.colourScheme)})` : ""}` : "none"} · ${p.keys} settings${p.unassigned ? ` (${p.unassigned} unassigned)` : ""}</p>
          <p class="muted">${size(p.bytes)} · Spotify ${short(p.versions.spotify)} · Spicetify ${esc(p.versions.spicetify ?? "?")}</p>
        </div>
        <div class="buttons">
          <button class="quiet" data-details="${esc(p.id)}">Details</button>
          <button class="quiet" data-export="${esc(p.id)}">Export</button>
          <button class="danger" data-delete="${esc(p.id)}" data-label="${esc(p.name ? `“${p.name}” (${when(p.createdAt)})` : `from ${when(p.createdAt)}`)}">Delete</button>
          <button class="go" data-restore="${esc(p.id)}">Restore</button>
        </div>
      </article>`,
    )
    .join("");
}

function keyTable(keys: KeyInfo[]) {
  if (!keys.length) return `<p class="muted">No settings.</p>`;
  return `<table><tbody>${keys.map((k) => `<tr><td><code>${esc(k.key)}</code></td><td class="num">${size(k.size)}</td></tr>`).join("")}</tbody></table>`;
}

async function showDetails(id: string) {
  const d = await invoke<Details>("profile_details", { id });
  $("details-title").textContent = d.name ? `${d.name} – ${when(d.createdAt)}` : `Profile from ${when(d.createdAt)}`;
  const exts = d.extensions
    .map(
      (e) => `<details class="ext">
        <summary><strong>${esc(e.name)}</strong> <span class="muted">${KIND[e.kind] ?? e.kind} · ${e.keys.length} settings${e.records ? ` · ${e.records} database records` : ""}${e.files.length ? ` · ${e.files.length} files` : ""}</span>${e.scanned ? "" : ` <span class="badge warn">code not readable</span>`}</summary>
        ${e.source ? `<p class="muted small">${esc(e.source)}</p>` : ""}
        ${keyTable(e.keyInfo)}
        ${e.databases.length ? `<p>Databases: ${e.databases.map((x) => `<code>${esc(x)}</code>`).join(", ")}</p>` : ""}
        ${e.files.length ? `<table><tbody>${e.files.map(([f, n, h]) => `<tr><td><code>${esc(f)}</code></td><td class="num">${size(n)}</td><td class="hash" title="SHA-256">${h.slice(0, 12)}…</td></tr>`).join("")}</tbody></table>` : ""}
      </details>`,
    )
    .join("");
  $("details-meta").innerHTML = `Spotify ${esc(d.versions.spotify ?? "?")} · Spicetify ${esc(d.versions.spicetify ?? "?")}${d.versions.marketplace ? ` · Marketplace ${esc(d.versions.marketplace)} (${d.marketplace.storage === "indexeddb" ? "IndexedDB" : d.marketplace.storage}, ${d.marketplace.keys.length} entries)` : ""}`;
  $("details-body").innerHTML = `
    ${exts || `<p class="muted">None.</p>`}
    <details class="ext"><summary><strong>Unassigned</strong> <span class="muted">${d.unassigned.length} settings no extension claimed – restored too</span></summary>${keyTable(d.unassigned)}</details>
    <details class="ext"><summary><strong>Spotify's own</strong> <span class="muted">${d.spotifyKeys.length} settings – kept, not restored unless you choose to</span></summary>${keyTable(d.spotifyKeys)}</details>
    ${d.notes.length ? `<h3 class="notes-head">Notes</h3><ul class="warnings">${d.notes.map((n) => `<li>${esc(n)}</li>`).join("")}</ul>` : ""}`;
  $("details-body").scrollTop = 0;
  $<HTMLDialogElement>("details").showModal();
}

function startBusy(title: string) {
  busy = true;
  $("busy-title").textContent = title;
  $("busy-step").textContent = "Starting…";
  $("busy-result").innerHTML = "";
  $("busy-close").hidden = true;
  $<HTMLDialogElement>("busy").showModal();
}
function endBusy(steps: Step[], headline: string) {
  busy = false;
  $("busy-step").textContent = headline;
  $("busy-result").innerHTML = steps.map((s) => `<li class="${s.ok ? "ok" : "fail"}"><strong>${esc(s.name)}</strong> ${esc(s.detail)}</li>`).join("");
  $("busy-close").hidden = false;
  refreshStatus();
  refreshProfiles();
}

async function capture() {
  if (busy) return;
  startBusy("Backing up");
  try {
    const s = await invoke<Summary>("capture");
    endBusy([{ name: "Saved", ok: true, detail: `${s.extensions} extensions and ${s.keys} settings.` }], "Done.");
  } catch (e) {
    endBusy([{ name: "Back up", ok: false, detail: String(e) }], "Nothing was saved.");
  }
}

let pending: string | null = null;
async function askRestore(id: string) {
  if (busy) return;
  const c = await invoke<Check>("restore_check", { id });
  pending = id;
  const items = [...c.blockers.map((b) => `<li class="fail">${esc(b)}</li>`), ...c.warnings.map((w) => `<li>${esc(w)} The extensions may not work the same on this version.</li>`)];
  $("confirm-warnings").innerHTML = items.join("");
  $<HTMLButtonElement>("confirm-go").disabled = c.blockers.length > 0;
  $<HTMLButtonElement>("confirm-go").textContent = c.warnings.length ? "Restore anyway" : "Restore";
  $<HTMLInputElement>("include-spotify").checked = false;
  $<HTMLDialogElement>("confirm").showModal();
}

async function restore() {
  const id = pending;
  if (!id) return;
  $<HTMLDialogElement>("confirm").close();
  startBusy("Restoring");
  try {
    const r = await invoke<Report>("restore", { id, options: { includeSpotify: $<HTMLInputElement>("include-spotify").checked, acknowledged: true } });
    endBusy(r.steps, r.ok ? "Restored. Spotify is running with your extensions and settings." : r.backupId ? `Stopped. Your setup from before is saved as a profile (${r.backupId}).` : "Stopped before changing anything.");
  } catch (e) {
    endBusy([{ name: "Restore", ok: false, detail: String(e) }], "Stopped.");
  }
}

let exporting: string | null = null;
function askExport(id: string) {
  if (busy) return;
  exporting = id;
  $<HTMLInputElement>("export-share").checked = false;
  $<HTMLDialogElement>("export").showModal();
}

async function exportProfile() {
  const id = exporting;
  if (!id) return;
  const share = $<HTMLInputElement>("export-share").checked;
  const path = await save({ defaultPath: `spicetify-pantry-${id}${share ? "-shared" : ""}.json`, filters: [{ name: "Spicetify Pantry profile", extensions: ["json"] }] });
  if (!path) return;
  $<HTMLDialogElement>("export").close();
  startBusy("Exporting");
  try {
    await invoke("export_profile", { id, path, share });
    endBusy([{ name: "Saved", ok: true, detail: `${path}${share ? " (for sharing)" : ""}` }], "Done.");
  } catch (e) {
    endBusy([{ name: "Export", ok: false, detail: String(e) }], "Nothing was saved.");
  }
}

async function importProfile() {
  if (busy) return;
  const path = await open({ multiple: false, directory: false, filters: [{ name: "Spicetify Pantry profile", extensions: ["json"] }] });
  if (!path) return;
  startBusy("Importing");
  try {
    const i = await invoke<Imported>("import_profile", { path });
    const theme = i.theme ? `${i.theme}${i.colourScheme ? ` (${i.colourScheme})` : ""}` : "no theme";
    endBusy(
      [{ name: i.alreadyThere ? "Already in your list" : "Imported", ok: true, detail: `${i.extensions} extensions, ${theme}${i.shared ? ", shared copy" : ""}.` }],
      i.alreadyThere ? "Nothing new to add." : "Done. Press Restore on it to set up this PC the same way.",
    );
  } catch (e) {
    endBusy([{ name: "Import", ok: false, detail: String(e) }], "Nothing was added.");
  }
}

let deleting: string | null = null;
function askDelete(id: string, label: string) {
  if (busy) return;
  deleting = id;
  $("delete-text").textContent = `Delete the profile ${label}? It goes to the Recycle Bin, so you can still get it back from there.`;
  $<HTMLDialogElement>("delete").showModal();
}

async function deleteProfile() {
  const id = deleting;
  if (!id) return;
  $<HTMLDialogElement>("delete").close();
  try {
    await invoke("delete_profile", { id });
  } catch (e) {
    startBusy("Deleting");
    endBusy([{ name: "Delete", ok: false, detail: String(e) }], "Nothing was deleted.");
  }
  refreshProfiles();
}

/** Pencil with its tip at the bottom left and the top end at the top right. */
const PENCIL = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.83 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/></svg>`;

/** The rename field that's open, if any. While it is, nothing else on the page reacts to clicks. */
let renaming: HTMLInputElement | null = null;

/** Turns the card's title into a text field. Only Enter (save) or Esc (cancel) closes it:
 *  Tab and clicks elsewhere keep it open and focused. */
function startRename(b: HTMLElement) {
  if (busy || renaming) return;
  const title = b.parentElement?.querySelector<HTMLElement>(".title");
  if (!title || title.querySelector("input")) return;
  const input = document.createElement("input");
  input.className = "rename-field";
  input.value = b.dataset.name ?? "";
  input.placeholder = b.dataset.placeholder ?? "";
  input.maxLength = 80;
  input.setAttribute("aria-label", "Profile name");
  const hint = document.createElement("span");
  hint.className = "rename-hint";
  hint.textContent = "Enter saves · Esc cancels";
  title.replaceChildren(input, hint);
  b.hidden = true;
  renaming = input;
  input.focus();
  input.select();
  let done = false;
  const finish = async (save: boolean) => {
    if (done) return;
    done = true;
    renaming = null;
    if (save && input.value.trim() !== (b.dataset.name ?? "")) {
      try {
        await invoke("rename_profile", { id: b.dataset.rename, name: input.value });
      } catch (e) {
        startBusy("Renaming");
        endBusy([{ name: "Rename", ok: false, detail: String(e) }], "The name wasn't changed.");
      }
    }
    refreshProfiles();
  };
  input.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") finish(true);
    else if (ev.key === "Escape") {
      ev.preventDefault();
      finish(false);
    } else if (ev.key === "Tab") ev.preventDefault();
  });
  input.addEventListener("blur", () => !done && setTimeout(() => !done && input.focus()));
}

async function selfTest() {
  if (busy) return;
  const out = $("selftest-result");
  out.textContent = "Running…";
  busy = true;
  try {
    const t = await invoke<SelfTest>("self_test");
    await writeText(t.clipboard);
    out.textContent = `${t.summary} – copied`;
    out.title = t.results.map((r) => `#${r.id} ${r.status}: ${r.name} – ${r.detail}`).join("\n");
  } catch (e) {
    out.textContent = String(e);
  } finally {
    busy = false;
    refreshStatus();
  }
}

window.addEventListener("DOMContentLoaded", () => {
  listen<string>("progress", (e) => {
    if (busy) $("busy-step").textContent = e.payload;
  });
  $("capture").addEventListener("click", capture);
  $("confirm-go").addEventListener("click", restore);
  $("selftest").addEventListener("click", selfTest);
  $("open-folder").addEventListener("click", () => invoke("open_profiles_folder"));
  $("import").addEventListener("click", importProfile);
  $("export-go").addEventListener("click", exportProfile);
  $("delete-go").addEventListener("click", deleteProfile);
  $("close-spotify").addEventListener("click", closeSpotify);
  // While a rename is open, clicks outside its field do nothing and don't take the focus away.
  for (const type of ["mousedown", "click"]) {
    document.addEventListener(
      type,
      (ev) => {
        if (!renaming || ev.target === renaming) return;
        ev.preventDefault();
        ev.stopPropagation();
        renaming.focus();
      },
      true,
    );
  }
  document.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement;
    const b = t.closest<HTMLElement>("[data-details],[data-restore],[data-export],[data-delete],[data-rename],[data-close]");
    if (!b) return;
    if (b.dataset.details) showDetails(b.dataset.details);
    else if (b.dataset.restore) askRestore(b.dataset.restore);
    else if (b.dataset.export) askExport(b.dataset.export);
    else if (b.dataset.delete) askDelete(b.dataset.delete, b.dataset.label ?? b.dataset.delete);
    else if (b.dataset.rename) startRename(b);
    else if (b.hasAttribute("data-close")) b.closest("dialog")?.close();
  });
  $<HTMLDialogElement>("busy").addEventListener("cancel", (e) => busy && e.preventDefault());
  getVersion().then((v) => ($("version").textContent = `v${v}`));
  refreshStatus();
  refreshProfiles();
  setInterval(() => !busy && refreshStatus(), 5000);
});
