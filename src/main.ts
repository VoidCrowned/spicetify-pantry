import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open, save } from "@tauri-apps/plugin-dialog";

type Versions = { spotify: string | null; spicetify: string | null; marketplace: string | null };
type Theme = { name: string | null; colourScheme: string | null; source: string };
type Summary = {
  id: string; createdAt: string; reason: string; source: string; versions: Versions; theme: Theme;
  extensions: number; keys: number; unassigned: number; bytes: number; importedAt: string | null; shared: boolean;
};
type Imported = { id: string; alreadyThere: boolean; extensions: number; theme: string | null; colourScheme: string | null; shared: boolean };
type KeyInfo = { key: string; size: number };
type ExtensionDetails = {
  id: string; name: string; kind: string; source: string | null; keys: string[]; databases: string[];
  globals: string[]; scanned: boolean; keyInfo: KeyInfo[]; files: [string, number, string][]; records: number;
};
type Details = {
  id: string; createdAt: string; reason: string; source: string; versions: Versions; theme: Theme;
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
const when = (iso: string) => new Date(iso).toLocaleString("en-GB", { dateStyle: "medium", timeStyle: "short" });
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
          <h3>${when(p.createdAt)} ${p.reason === "before-restore" ? `<span class="badge">automatic, before a restore</span>` : ""}${p.source === "leveldb" ? `<span class="badge warn">read with Spotify closed</span>` : ""}${p.importedAt ? `<span class="badge">imported ${when(p.importedAt)}</span>` : ""}${p.shared ? `<span class="badge">shared copy</span>` : ""}</h3>
          <p class="muted">${p.extensions} extensions · ${p.theme.name ? `${esc(p.theme.name)}${p.theme.colourScheme ? ` (${esc(p.theme.colourScheme)})` : ""}` : "no theme"} · ${p.keys} settings${p.unassigned ? ` (${p.unassigned} unassigned)` : ""} · Spotify ${short(p.versions.spotify)}, Spicetify ${esc(p.versions.spicetify ?? "?")} · ${size(p.bytes)}</p>
        </div>
        <div class="buttons">
          <button class="quiet" data-details="${esc(p.id)}">Details</button>
          <button class="quiet" data-export="${esc(p.id)}">Export</button>
          <button class="danger" data-delete="${esc(p.id)}" data-when="${esc(when(p.createdAt))}">Delete</button>
          <button data-restore="${esc(p.id)}">Restore</button>
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
  $("details-title").textContent = `Profile from ${when(d.createdAt)}`;
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
  $("delete-text").textContent = `Delete the profile from ${label}? It goes to the Recycle Bin, so you can still get it back from there.`;
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
  document.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement;
    const b = t.closest<HTMLElement>("[data-details],[data-restore],[data-export],[data-delete],[data-close]");
    if (!b) return;
    if (b.dataset.details) showDetails(b.dataset.details);
    else if (b.dataset.restore) askRestore(b.dataset.restore);
    else if (b.dataset.export) askExport(b.dataset.export);
    else if (b.dataset.delete) askDelete(b.dataset.delete, b.dataset.when ?? b.dataset.delete);
    else if (b.hasAttribute("data-close")) b.closest("dialog")?.close();
  });
  $<HTMLDialogElement>("busy").addEventListener("cancel", (e) => busy && e.preventDefault());
  refreshStatus();
  refreshProfiles();
  setInterval(() => !busy && refreshStatus(), 5000);
});
