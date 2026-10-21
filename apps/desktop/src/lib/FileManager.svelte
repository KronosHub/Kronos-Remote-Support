<script lang="ts">
  // File manager of a session we view (ADR 0053): this computer's folders on the left, the
  // partner's on the right; both open on the home folder of the user running the app.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  let { session }: { session: number } = $props();

  type FsEntry = {
    name: string;
    path: string;
    dir: boolean;
    size: number;
    modified: number | null;
    /** Missing on file systems without it and from hosts before 1.9.2. */
    created?: number | null;
    hidden: boolean;
  };
  type SortKey = "name" | "type" | "size" | "modified" | "created";
  type Sort = { key: SortKey; desc: boolean };
  type Listing = {
    path: string;
    parent: string | null;
    home: string | null;
    roots: string[];
    entries: FsEntry[];
    truncated: boolean;
  };
  type Side = "local" | "remote";
  type Pane = {
    listing: Listing | null;
    loading: boolean;
    error: string;
    /** Paths of the selected entries. */
    selected: string[];
    /** Last entry clicked, for Shift+click ranges. */
    anchor: string | null;
    /** Typed in the address bar. */
    address: string;
    /** Name of the folder being created, while its field is open. */
    newFolder: string | null;
    sort: Sort;
  };
  type Copy = {
    req: number;
    upload: boolean;
    label: string;
    dest: string;
    done: number;
    total: number;
    speed: number;
    state: "running" | "done" | "error" | "cancelled";
    message: string;
  };
  type Event =
    | { kind: "fs_progress"; session: number; req: number; done: number; total: number; bytes_per_sec: number }
    | { kind: "fs_done"; session: number; req: number; upload: boolean; names: string[]; error: string | null }
    | { kind: "view_closed"; session: number; reason: string };

  const emptyPane = (): Pane => ({
    listing: null,
    loading: false,
    error: "",
    selected: [],
    anchor: null,
    address: "",
    newFolder: null,
    sort: loadSort(),
  });
  const SORT_KEY = "fm-sort";
  function loadSort(): Sort {
    try {
      const saved = JSON.parse(localStorage.getItem(SORT_KEY) ?? "null");
      if (saved && ["name", "type", "size", "modified", "created"].includes(saved.key)) {
        return { key: saved.key, desc: !!saved.desc };
      }
    } catch {
      // No storage: the default order.
    }
    return { key: "name", desc: false };
  }
  /** Same column again: reverse; another one: ascending (dates and sizes: newest/largest first). */
  function setSort(side: Side, key: SortKey, desc?: boolean) {
    const pane = panes[side];
    pane.sort = {
      key,
      desc: desc ?? (pane.sort.key === key ? !pane.sort.desc : key === "modified" || key === "created" || key === "size"),
    };
    try {
      localStorage.setItem(SORT_KEY, JSON.stringify(pane.sort));
    } catch {
      // Not remembered.
    }
  }
  const collator = new Intl.Collator("it", { numeric: true, sensitivity: "base" });
  const extension = (e: FsEntry) => {
    const dot = e.name.lastIndexOf(".");
    return e.dir || dot <= 0 ? "" : e.name.slice(dot + 1).toLowerCase();
  };
  /** Folders always first; ties broken by name. */
  function sorted(entries: FsEntry[], sort: Sort): FsEntry[] {
    const sign = sort.desc ? -1 : 1;
    const by = (a: FsEntry, b: FsEntry): number => {
      switch (sort.key) {
        case "type":
          return collator.compare(extension(a), extension(b));
        case "size":
          return a.size - b.size;
        case "modified":
          return (a.modified ?? 0) - (b.modified ?? 0);
        case "created":
          return (a.created ?? 0) - (b.created ?? 0);
        default:
          return 0;
      }
    };
    return [...entries].sort(
      (a, b) => Number(b.dir) - Number(a.dir) || sign * by(a, b) || sign * collator.compare(a.name, b.name),
    );
  }
  const sortLabels: Record<SortKey, string> = {
    name: "Nome",
    type: "Tipo",
    size: "Dimensione",
    modified: "Data di modifica",
    created: "Data di creazione",
  };
  let panes = $state<Record<Side, Pane>>({ local: emptyPane(), remote: emptyPane() });
  let peer = $state("");
  let showHidden = $state(false);
  let copies = $state<Copy[]>([]);
  /** Why the session ended, once it has. */
  let ended = $state<string | null>(null);
  const running = $derived(copies.some((c) => c.state === "running"));

  function visible(side: Side): FsEntry[] {
    const entries = panes[side].listing?.entries ?? [];
    return sorted(showHidden ? entries : entries.filter((e) => !e.hidden), panes[side].sort);
  }

  async function load(side: Side, path: string | null = null, retries = 0) {
    const pane = panes[side];
    pane.loading = true;
    try {
      const listing =
        side === "local"
          ? await invoke<Listing>("fs_local_list", { path })
          : await invoke<Listing>("fs_remote_list", { session, path });
      pane.listing = listing;
      pane.address = listing.path;
      pane.selected = [];
      pane.anchor = null;
      pane.error = "";
    } catch (e) {
      const message = String(e);
      // The partner's channel opens a moment after the session: try again shortly.
      if (side === "remote" && retries < 3 && message.includes("non permette ancora")) {
        setTimeout(() => load(side, path, retries + 1), 1000);
        return;
      }
      pane.error = message;
      if (pane.listing) pane.address = pane.listing.path;
    } finally {
      pane.loading = false;
    }
  }

  const separator = (path: string) => (path.includes("\\") && !path.includes("/") ? "\\" : "/");
  function join(dir: string, name: string): string {
    const sep = separator(dir);
    return dir.endsWith(sep) ? dir + name : dir + sep + name;
  }

  function click(side: Side, entry: FsEntry, event: MouseEvent) {
    const pane = panes[side];
    if (event.shiftKey && pane.anchor) {
      const list = visible(side).map((e) => e.path);
      const [a, b] = [list.indexOf(pane.anchor), list.indexOf(entry.path)].sort((x, y) => x - y);
      if (a >= 0) {
        pane.selected = list.slice(a, b + 1);
        return;
      }
    }
    if (event.metaKey || event.ctrlKey) {
      pane.selected = pane.selected.includes(entry.path)
        ? pane.selected.filter((p) => p !== entry.path)
        : [...pane.selected, entry.path];
    } else {
      pane.selected = [entry.path];
    }
    pane.anchor = entry.path;
  }

  function open(side: Side, entry: FsEntry) {
    if (entry.dir) load(side, entry.path);
  }

  function keydown(side: Side, event: KeyboardEvent) {
    const pane = panes[side];
    if (event.key === "Enter") {
      const entry = visible(side).find((e) => pane.selected.length === 1 && e.path === pane.selected[0]);
      if (entry) open(side, entry);
    } else if (event.key === "Backspace" && pane.listing?.parent) {
      load(side, pane.listing.parent);
    } else if ((event.metaKey || event.ctrlKey) && event.key === "a") {
      event.preventDefault();
      pane.selected = visible(side).map((e) => e.path);
    }
  }

  async function createFolder(side: Side, event: SubmitEvent) {
    event.preventDefault();
    const pane = panes[side];
    const name = pane.newFolder?.trim();
    if (!name || !pane.listing) return;
    if (/[\\/]/.test(name)) {
      pane.error = "il nome non può contenere / o \\";
      return;
    }
    const path = join(pane.listing.path, name);
    try {
      if (side === "local") await invoke("fs_local_mkdir", { path });
      else await invoke("fs_remote_mkdir", { session, path });
      pane.newFolder = null;
      await load(side, pane.listing.path);
      pane.selected = [path];
    } catch (e) {
      pane.error = String(e);
    }
  }

  /** A copy waiting for the user's choice: some names already exist in the destination. */
  let conflict = $state<{ from: Side; existing: string[]; dest: string } | null>(null);

  /** Copies the selection of one side into the folder shown on the other; asks first if
   * items with the same names are there (names compared ignoring case, as on Windows and
   * macOS). */
  function copy(from: Side) {
    const to: Side = from === "local" ? "remote" : "local";
    const source = panes[from];
    const dest = panes[to].listing?.path;
    if (!dest || source.selected.length === 0) return;
    const taken = new Set((panes[to].listing?.entries ?? []).map((e) => e.name.toLowerCase()));
    const existing = (source.listing?.entries ?? [])
      .filter((e) => source.selected.includes(e.path) && taken.has(e.name.toLowerCase()))
      .map((e) => e.name);
    if (existing.length > 0) {
      conflict = { from, existing, dest };
      return;
    }
    startCopy(from, false);
  }

  function resolveConflict(replace: boolean | null) {
    const pending = conflict;
    conflict = null;
    if (pending && replace !== null) startCopy(pending.from, replace);
  }

  async function startCopy(from: Side, replace: boolean) {
    const to: Side = from === "local" ? "remote" : "local";
    const source = panes[from];
    const dest = panes[to].listing?.path;
    if (!dest || source.selected.length === 0) return;
    const names = (source.listing?.entries ?? [])
      .filter((e) => source.selected.includes(e.path))
      .map((e) => e.name);
    const label = names.length === 1 ? names[0] : `${names.length} elementi`;
    try {
      const req = await invoke<number>("fs_copy", {
        session,
        upload: from === "local",
        paths: source.selected,
        dest,
        replace,
      });
      const started: Copy = {
        req, upload: from === "local", label, dest, done: 0, total: 0, speed: 0, state: "running", message: "",
      };
      copies = [started, ...copies].slice(0, 20);
    } catch (e) {
      source.error = String(e);
    }
  }

  async function cancel(c: Copy) {
    await invoke("fs_cancel", { session, req: c.req }).catch(() => {});
  }

  function handle(event: Event) {
    if (event.session !== session) return;
    if (event.kind === "view_closed") {
      ended = event.reason || "sessione terminata";
      return;
    }
    const c = copies.find((c) => c.req === event.req);
    if (!c) return;
    if (event.kind === "fs_progress") {
      c.done = event.done;
      c.total = event.total;
      c.speed = event.bytes_per_sec;
      return;
    }
    if (event.error) {
      c.state = event.error === "copia annullata" ? "cancelled" : "error";
      c.message = event.error;
    } else {
      c.state = "done";
      c.done = c.total;
      c.message = event.names.length === 1 ? `salvato come ${event.names[0]}` : `${event.names.length} elementi copiati`;
      // Show the new files where they landed.
      const side: Side = event.upload ? "remote" : "local";
      if (panes[side].listing?.path === c.dest) {
        load(side, c.dest).then(() => {
          panes[side].selected = (panes[side].listing?.entries ?? [])
            .filter((e) => event.names.includes(e.name))
            .map((e) => e.path);
        });
      }
    }
  }

  function size(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit++;
    }
    return `${value.toFixed(value < 10 ? 1 : 0).replace(".", ",")} ${units[unit]}`;
  }
  const date = (secs: number | null) =>
    secs === null ? "" : new Date(secs * 1000).toLocaleString("it-IT", { dateStyle: "short", timeStyle: "short" });
  const percent = (c: Copy) => (c.total > 0 ? Math.min(100, Math.round((c.done / c.total) * 100)) : 0);

  onMount(() => {
    const unlisten = listen<Event>("service", (e) => handle(e.payload));
    getCurrentWindow()
      .title()
      .then((t) => (peer = t.replace(/^Trasferimento file — /, "")));
    load("local");
    load("remote");
    return () => {
      unlisten.then((f) => f());
    };
  });
</script>

<div class="fm">
  {#if ended}
    <div class="banner">Sessione terminata ({ended}): non puoi più copiare file con il partner.</div>
  {/if}
  <div class="panes">
    {#each ["local", "remote"] as const as side (side)}
      {@const pane = panes[side]}
      <section class="pane" aria-label={side === "local" ? "Questo computer" : "Computer remoto"}>
        <header>
          <h2>
            {#if side === "local"}Questo computer{:else}Computer remoto{#if peer}<span class="muted">{` · ${peer}`}</span>{/if}{/if}
          </h2>
          <div class="nav">
            <button title="Cartella superiore" aria-label="Cartella superiore"
              disabled={!pane.listing?.parent || (side === "remote" && !!ended)}
              onclick={() => pane.listing?.parent && load(side, pane.listing.parent)}>
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.2"
                stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5M5 12l7-7 7 7"/></svg>
            </button>
            <button title="Cartella home" aria-label="Cartella home" disabled={side === "remote" && !!ended}
              onclick={() => load(side, pane.listing?.home ?? null)}>
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"
                stroke-linejoin="round" aria-hidden="true"><path d="M3 11 12 4l9 7"/><path d="M5 10v10h14V10"/></svg>
            </button>
            {#if (pane.listing?.roots.length ?? 0) > 1}
              <select title="Disco" value={pane.listing?.roots.find((r) => pane.listing?.path.startsWith(r)) ?? ""}
                onchange={(e) => load(side, e.currentTarget.value)}>
                {#each pane.listing?.roots ?? [] as root (root)}<option value={root}>{root}</option>{/each}
              </select>
            {/if}
            <form class="address" onsubmit={(e) => { e.preventDefault(); load(side, pane.address); }}>
              <input class="mono" spellcheck="false" autocomplete="off" aria-label="Percorso"
                disabled={side === "remote" && !!ended} bind:value={pane.address} />
            </form>
            <button title="Aggiorna" aria-label="Aggiorna" disabled={!pane.listing || (side === "remote" && !!ended)}
              onclick={() => load(side, pane.listing?.path ?? null)}>
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.2"
                stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/></svg>
            </button>
            <button title="Nuova cartella" disabled={!pane.listing || (side === "remote" && !!ended)}
              onclick={() => (pane.newFolder = "")}>Nuova cartella</button>
          </div>
          {#if pane.newFolder !== null}
            <form class="new-folder" onsubmit={(e) => createFolder(side, e)}>
              <!-- svelte-ignore a11y_autofocus -->
              <input placeholder="Nome della cartella" autofocus bind:value={pane.newFolder}
                onkeydown={(e) => e.key === "Escape" && (pane.newFolder = null)} />
              <button class="primary" type="submit" disabled={!pane.newFolder.trim()}>Crea</button>
              <button type="button" onclick={() => (pane.newFolder = null)}>Annulla</button>
            </form>
          {/if}
          {#if pane.error}<p class="error">{pane.error}</p>{/if}
        </header>
        <div class="columns" role="toolbar" aria-label="Ordinamento">
          <span></span>
          <button class="col" class:active={pane.sort.key === "name"} onclick={() => setSort(side, "name")}
            title="Ordina per nome">Nome{pane.sort.key === "name" ? (pane.sort.desc ? " ↓" : " ↑") : ""}</button>
          <button class="col right" class:active={pane.sort.key === "size"} onclick={() => setSort(side, "size")}
            title="Ordina per dimensione">Dimensione{pane.sort.key === "size" ? (pane.sort.desc ? " ↓" : " ↑") : ""}</button>
          <button class="col right" class:active={pane.sort.key === "modified" || pane.sort.key === "created"}
            onclick={() => setSort(side, pane.sort.key === "created" ? "created" : "modified")}
            title="Ordina per data">{pane.sort.key === "created" ? "Creato" : "Modificato"}{pane.sort.key === "modified" || pane.sort.key === "created" ? (pane.sort.desc ? " ↓" : " ↑") : ""}</button>
        </div>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="list" class:loading={pane.loading} tabindex="0" role="listbox" aria-multiselectable="true"
          onkeydown={(e) => keydown(side, e)}>
          {#if pane.listing}
            {#each visible(side) as entry (entry.path)}
              <div class="row" class:selected={pane.selected.includes(entry.path)} class:hidden={entry.hidden}
                role="option" aria-selected={pane.selected.includes(entry.path)} tabindex="-1"
                onclick={(e) => click(side, entry, e)} ondblclick={() => open(side, entry)}
                onkeydown={() => {}}>
                <span class="icon" aria-hidden="true">
                  {#if entry.dir}
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" opacity="0.85">
                      <path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
                  {:else}
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8"
                      stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/></svg>
                  {/if}
                </span>
                <span class="name" title={entry.name}>{entry.name}</span>
                <span class="size muted">{entry.dir ? "" : size(entry.size)}</span>
                <span class="date muted">{date(pane.sort.key === "created" ? (entry.created ?? null) : entry.modified)}</span>
              </div>
            {:else}
              <p class="hint">Cartella vuota</p>
            {/each}
            {#if pane.listing.truncated}
              <p class="hint">Mostrati solo i primi 5000 elementi.</p>
            {/if}
          {:else if pane.loading}
            <p class="hint">Caricamento…</p>
          {/if}
        </div>
        <footer class="muted small">
          <span>
            {#if pane.selected.length > 0}
              {pane.selected.length} {pane.selected.length === 1 ? "selezionato" : "selezionati"}
            {:else if pane.listing}
              {visible(side).length} elementi
            {/if}
          </span>
          <span class="sorting">
            Ordina per
            <select class="sort" aria-label="Ordina per" value={pane.sort.key}
              onchange={(e) => setSort(side, e.currentTarget.value as SortKey, pane.sort.desc)}>
              {#each Object.entries(sortLabels) as [key, label] (key)}<option value={key}>{label}</option>{/each}
            </select>
            <button class="col dir" title={pane.sort.desc ? "Decrescente: clic per crescente" : "Crescente: clic per decrescente"}
              aria-label="Inverti ordine" onclick={() => setSort(side, pane.sort.key, !pane.sort.desc)}>
              {pane.sort.desc ? "↓ decrescente" : "↑ crescente"}</button>
          </span>
        </footer>
      </section>
      {#if side === "local"}
        <div class="actions">
          <button class="primary" title="Copia la selezione di questo computer nella cartella aperta del partner"
            disabled={!!ended || panes.local.selected.length === 0 || !panes.remote.listing}
            onclick={() => copy("local")}>Invia →</button>
          <button class="primary" title="Copia la selezione del partner nella cartella aperta di questo computer"
            disabled={!!ended || panes.remote.selected.length === 0 || !panes.local.listing}
            onclick={() => copy("remote")}>← Ricevi</button>
        </div>
      {/if}
    {/each}
  </div>
  <div class="bottom">
    <label class="check">
      <input type="checkbox" bind:checked={showHidden} />
      Mostra file nascosti
    </label>
    <span class="muted small">Doppio clic per aprire una cartella · Ctrl/⌘ o Maiusc per selezionarne più di uno</span>
    {#if running}<span class="muted small">· copia in corso</span>{/if}
  </div>
  {#if copies.length > 0}
    <ul class="copies">
      {#each copies as c (c.req)}
        <li>
          <span class="direction" title={c.upload ? "Verso il partner" : "Dal partner"}>{c.upload ? "→" : "←"}</span>
          <span class="copy-label" title={`${c.label} → ${c.dest}`}>{c.label}</span>
          {#if c.state === "running"}
            <progress max="100" value={percent(c)}></progress>
            <span class="muted small nowrap">
              {c.total > 0 ? `${size(c.done)} di ${size(c.total)}` : "preparazione…"}{c.speed > 0 ? ` · ${size(c.speed)}/s` : ""}
            </span>
            <button class="small-btn" onclick={() => cancel(c)}>Annulla</button>
          {:else}
            <span class="state {c.state}">{c.state === "done" ? "Completata" : c.state === "cancelled" ? "Annullata" : "Errore"}</span>
            <span class="muted small message" title={c.message}>{c.message}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if conflict}
    <div class="backdrop">
      <div class="modal" role="alertdialog" aria-labelledby="conflict-title">
        <h2 id="conflict-title">
          {conflict.existing.length === 1 ? "Il file esiste già" : `${conflict.existing.length} elementi esistono già`}
        </h2>
        <p>
          {#if conflict.existing.length === 1}
            <strong>{conflict.existing[0]}</strong> c'è già in
          {:else}
            {conflict.existing.slice(0, 5).join(", ")}{conflict.existing.length > 5 ? "…" : ""}: già presenti in
          {/if}
          <span class="mono">{conflict.dest}</span>.
        </p>
        <p class="muted small">
          Sovrascrivi sostituisce gli elementi con lo stesso nome (una cartella viene sostituita per intero).
          Crea una copia salva i nuovi con un altro nome, per esempio "nome (2)".
        </p>
        <div class="modal-actions">
          <button onclick={() => resolveConflict(null)}>Annulla</button>
          <button onclick={() => resolveConflict(false)}>Crea una copia</button>
          <button class="danger" onclick={() => resolveConflict(true)}>Sovrascrivi</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .fm {
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    padding: 12px;
    gap: 10px;
    background: var(--bg);
    color: var(--text);
  }
  .banner {
    padding: 8px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--warn) 18%, transparent);
    color: var(--warn);
    font-size: 13px;
  }
  .panes {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    gap: 10px;
  }
  .pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }
  header {
    padding: 10px 10px 8px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    font-size: 14px;
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .nav,
  .new-folder {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .address {
    flex: 1;
    min-width: 0;
  }
  input {
    width: 100%;
    box-sizing: border-box;
    padding: 5px 8px;
    font: inherit;
    font-size: 13px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
  input:focus,
  .list:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  button,
  select {
    font: inherit;
    font-size: 13px;
    padding: 5px 9px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    white-space: nowrap;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }
  .columns {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) 76px 116px;
    gap: 8px;
    align-items: center;
    padding: 4px 12px 4px 12px;
    border-bottom: 1px solid var(--border);
  }
  .col {
    border: none;
    background: none;
    padding: 2px 0;
    font-size: 12px;
    color: var(--muted);
  }
  .col.right {
    justify-content: flex-end;
  }
  .col.active {
    color: var(--text);
    font-weight: 600;
  }
  .col.dir {
    padding: 2px 4px;
  }
  .sort {
    font-size: 12px;
    padding: 2px 4px;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
  }
  .modal {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px 18px;
    max-width: 460px;
    width: 100%;
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .modal h2 {
    font-size: 15px;
    margin-bottom: 8px;
    white-space: normal;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  button.danger {
    background: var(--err);
    border-color: var(--err);
    color: #fff;
    font-weight: 600;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 4px;
    outline: none;
  }
  .list.loading {
    opacity: 0.6;
  }
  .row {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) 76px 116px;
    gap: 8px;
    align-items: center;
    padding: 4px 8px;
    border-radius: 6px;
    font-size: 13px;
    cursor: default;
    user-select: none;
  }
  .row:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .row.selected {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .row.hidden .name {
    opacity: 0.6;
  }
  .icon {
    display: flex;
    color: var(--accent);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size,
  .date {
    font-size: 12px;
    text-align: right;
    white-space: nowrap;
  }
  footer {
    padding: 4px 12px;
    border-top: 1px solid var(--border);
    min-height: 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }
  .sorting {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 10px;
  }
  .bottom {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    cursor: pointer;
  }
  .check input {
    width: auto;
  }
  .copies {
    list-style: none;
    margin: 0;
    padding: 6px 10px;
    max-height: 140px;
    overflow: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .copies li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
    font-size: 13px;
  }
  .direction {
    font-weight: 700;
    color: var(--accent);
  }
  .copy-label {
    max-width: 260px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  progress {
    flex: 1;
    min-width: 80px;
    accent-color: var(--accent);
  }
  .state.done {
    color: var(--ok);
  }
  .state.error {
    color: var(--err);
  }
  .state.cancelled {
    color: var(--muted);
  }
  .message {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .small-btn {
    padding: 3px 8px;
    font-size: 12px;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 12px;
  }
  .nowrap {
    white-space: nowrap;
  }
  .hint {
    color: var(--muted);
    font-size: 13px;
    padding: 8px;
    margin: 0;
  }
  .error {
    color: var(--err);
    font-size: 12px;
    margin: 0;
  }
</style>
