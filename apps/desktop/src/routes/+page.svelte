<script lang="ts">
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import { RemoteVideo, webCodecsSupported } from "$lib/remote-video";
  import FileManager from "$lib/FileManager.svelte";

  type Link = "connecting" | "online" | "offline";
  type Status = {
    link: Link;
    id: string | null;
    password: string;
    unattended: boolean;
    error: string | null;
  };
  type Device = { id: string; name: string; group: string | null };
  /** A folder of saved devices; folders nest through `parent`. */
  type Group = { id: string; name: string; parent: string | null; collapsed?: boolean };
  type DeviceTree = { devices: Device[]; groups: Group[] };
  type UiEvent =
    | ({ kind: "status" } & Status)
    | { kind: "presence"; devices: Record<string, boolean> | null }
    | { kind: "log"; level: "info" | "warn" | "error"; message: string }
    | { kind: "approval_request"; session: number; peer: string; timeout_secs: number }
    | { kind: "approval_closed"; session: number }
    | { kind: "view_opened"; session: number; peer: string; peer_name: string | null }
    | { kind: "view_size"; session: number; width: number; height: number; hardware: boolean | null }
    | { kind: "view_rtt"; session: number; ms: number }
    | { kind: "view_resync"; session: number }
    | {
        kind: "view_stats";
        session: number;
        rx_kbps: number;
        tx_kbps: number;
        cpu: number | null;
        ram_mb: number | null;
      }
    | { kind: "view_partner_usage"; session: number; cpu: number; ram_mb: number }
    | { kind: "view_quality"; session: number; kbps: number; fps: number; limited: boolean }
    | { kind: "view_clipboard"; session: number; enabled: boolean }
    | { kind: "view_audio"; session: number; enabled: boolean }
    | { kind: "view_input_blocked"; session: number; blocked: boolean }
    | { kind: "view_silent"; session: number; silent: boolean }
    | { kind: "view_resolution_adjustable"; session: number }
    | { kind: "hosting_input_blocked"; session: number; blocked: boolean }
    | { kind: "view_notice"; session: number; message: string }
    | { kind: "view_chat"; session: number; text: string }
    | { kind: "host_chat"; session: number; peer: string; text: string }
    | { kind: "host_chat_sent"; session: number; text: string; window: string }
    | {
        kind: "transfer";
        session: number;
        sending: boolean;
        done: number;
        total: number;
        bytes_per_sec: number;
        finished: boolean;
      }
    | { kind: "view_control_unavailable"; session: number; message: string }
    | { kind: "view_no_screen"; session: number; message: string }
    | { kind: "view_displays"; session: number; displays: DisplayInfo[]; current: number }
    | {
        kind: "view_closed";
        session: number;
        reason: string;
        updating: boolean;
        reconnect: boolean;
        duration_secs: number;
      }
    | { kind: "reconnect_failed"; peer: string }
    | { kind: "hosting_started"; session: number; peer: string }
    | { kind: "hosting_ended"; session: number; peer: string; duration_secs: number }
    | { kind: "connection_path"; session: number; path: Path | null };
  type Path = "lan" | "direct" | "relay";
  /** The partner screen we are watching. */
  type View = {
    session: number;
    peer: string;
    width: number | null;
    height: number | null;
    /** The partner encodes the video on its video chip (true) or processor (false); null: unknown. */
    hardware: boolean | null;
    rtt: number | null;
    fps: number | null;
    waiting: boolean;
    error: string | null;
    /** The partner has no screen to share yet (no monitor...): why, while the session stays open. */
    noScreen: string | null;
    /** Send mouse and keyboard to the partner. */
    control: boolean;
    controlError: string | null;
    displays: DisplayInfo[];
    currentDisplay: number | null;
    /** When the picture was resynchronised (ms timestamps, last minute only). */
    resyncs: number[];
    /** The first resync is the decoder starting up: not shown. */
    resyncStarted: boolean;
    /** HD: the partner sends up to its full size (on) or at most 1080p (off). */
    hd: boolean;
    /** The partner's sound plays here (off by default). */
    audio: boolean;
    /** The partner's own mouse and keyboard are blocked. */
    inputBlocked: boolean;
    /** The partner may set its screen to this window's size ("solo da remoto", ADR 0061). */
    resizable: boolean;
    /** Name the partner set in their app, if any. */
    peerName: string | null;
    transfer: Transfer | null;
    path: Path | null;
    /** Session traffic, kbit/s. */
    net: { rx: number; tx: number } | null;
    /** CPU (% of the machine) and memory (MB) of this app and of the partner's. */
    local: { cpu: number; ram: number } | null;
    partner: { cpu: number; ram: number } | null;
    /** Set while the partner lowered the quality for lack of bandwidth. */
    limited: { kbps: number; fps: number } | null;
    /** Clipboard shared with the partner (null until known). */
    clipboard: boolean | null;
    /** Transient message (files received on the clipboard...). */
    notice: string | null;
    chat: ChatMsg[];
    /** The chat drawer is open on the right edge. */
    showChat: boolean;
    unread: number;
    /** The drawer was opened at least once: when closed, its tab stays on the edge. */
    chatUsed: boolean;
    /** The user hid the drawer: new messages light up the tab instead of opening it. */
    chatDismissed: boolean;
    /** Start of the last message received while the drawer is hidden, shown next to the tab. */
    chatPeek: string | null;
    /** Why the session ended; the window stays open to show it. */
    ended: string | null;
    /** Connecting again by itself (see `Reconnect` in service.rs): the window closes when
     * the new session to the same device opens. */
    reconnecting: boolean;
    /** The partner has not answered for a few seconds. */
    silent: boolean;
  };
  type InputEvent =
    | { k: "mouse_move"; x: number; y: number }
    | { k: "mouse_button"; button: string; down: boolean }
    | { k: "wheel"; dx: number; dy: number }
    | { k: "key"; code: string; down: boolean };
  type ChatMsg = { mine: boolean; text: string; time: string };
  /** A clipboard file transfer under way (progress bar). */
  type Transfer = { sending: boolean; done: number; total: number; rate: number };
  type Hosting = {
    session: number;
    peer: string;
    path: Path | null;
    chat: ChatMsg[];
    showChat: boolean;
    unread: number;
    /** The partner blocked this computer's own mouse and keyboard. */
    blocked: boolean;
    transfer: Transfer | null;
  };
  type DisplayInfo = { id: number; name: string; width: number; height: number; primary: boolean };
  type LogLine = { time: string; level: string; message: string };
  type Approval = { session: number; peer: string; deadline: number; total: number };

  let status = $state<Status>({
    link: "connecting",
    id: null,
    password: "",
    unattended: false,
    error: null,
  });
  let devices = $state<Device[]>([]);
  let groups = $state<Group[]>([]);
  function setTree(tree: DeviceTree) {
    devices = tree.devices;
    groups = tree.groups;
  }
  async function reloadDevices() {
    setTree(await invoke<DeviceTree>("list_device_tree"));
  }
  /** Folders inside `parent` (null: the top of the list). */
  function childGroups(parent: string | null): Group[] {
    return groups.filter((g) => (g.parent ?? null) === parent);
  }
  /** Devices directly in `parent`; one in a folder that no longer exists is at the top. */
  function devicesIn(parent: string | null): Device[] {
    return devices.filter((d) => {
      const group = d.group && groups.some((g) => g.id === d.group) ? d.group : null;
      return group === parent;
    });
  }
  /** Devices in a folder and its subfolders, and how many are online. */
  function folderCount(id: string): { total: number; online: number } {
    let total = 0;
    let online = 0;
    for (const d of devicesIn(id)) {
      total++;
      if (deviceOnline(d)) online++;
    }
    for (const g of childGroups(id)) {
      const c = folderCount(g.id);
      total += c.total;
      online += c.online;
    }
    return { total, online };
  }
  /** Every folder in tree order, indented, for the "move to" menus; `except` and the folders
   *  inside it are left out (a folder cannot go inside itself). */
  function folderOptions(except: string | null = null): { id: string; label: string }[] {
    const out: { id: string; label: string }[] = [];
    const walk = (parent: string | null, depth: number) => {
      for (const g of childGroups(parent)) {
        if (g.id === except) continue;
        out.push({ id: g.id, label: "\u00a0\u00a0\u00a0".repeat(depth) + g.name });
        walk(g.id, depth + 1);
      }
    };
    walk(null, 0);
    return out;
  }
  /** New folder (`id` null) or one being renamed/moved. */
  let groupDialog = $state<{ id: string | null; name: string; parent: string | null; error: string } | null>(null);
  async function saveGroup(event: SubmitEvent) {
    event.preventDefault();
    if (!groupDialog) return;
    const { id, name, parent } = groupDialog;
    try {
      if (id === null) {
        setTree(await invoke<DeviceTree>("create_group", { name, parent }));
      } else {
        const old = groups.find((g) => g.id === id);
        if (old && old.name !== name.trim()) setTree(await invoke<DeviceTree>("rename_group", { id, name }));
        if (old && (old.parent ?? null) !== parent) setTree(await invoke<DeviceTree>("move_group", { id, parent }));
      }
      groupDialog = null;
    } catch (e) {
      if (groupDialog) groupDialog.error = String(e);
    }
  }
  function toggleGroup(g: Group) {
    g.collapsed = !g.collapsed;
    invoke("set_group_collapsed", { id: g.id, collapsed: g.collapsed }).catch((e) => addLog("error", String(e)));
  }
  async function deleteGroup(g: Group) {
    const message = `Eliminare la cartella "${g.name}"? I dispositivi e le cartelle che contiene passano alla cartella superiore.`;
    if (!(await askConfirm(message, "Elimina"))) return;
    try {
      setTree(await invoke<DeviceTree>("delete_group", { id: g.id }));
    } catch (e) {
      addLog("error", String(e));
    }
  }
  /** "Organizza": the only mode where devices can be picked and moved, so a stray click in
   *  the normal list never moves anything. */
  let organizing = $state(false);
  let picked = $state<string[]>([]);
  /** The "Sposta in…" dialog: the chosen folder ("" is the top of the list). */
  let moveDialog = $state<{ target: string } | null>(null);
  /** The last move, undoable for a few seconds: each device's previous folder. */
  let lastMove = $state<{ message: string; previous: { id: string; group: string | null }[] } | null>(null);
  let lastMoveTimer: ReturnType<typeof setTimeout> | undefined;
  function startOrganizing() {
    organizing = true;
    picked = [];
  }
  function stopOrganizing() {
    organizing = false;
    picked = [];
    moveDialog = null;
  }
  function togglePicked(d: Device) {
    picked = picked.includes(d.id) ? picked.filter((id) => id !== d.id) : [...picked, d.id];
  }
  /** Every device under a folder, subfolders included. */
  function devicesUnder(id: string): Device[] {
    return [...devicesIn(id), ...childGroups(id).flatMap((g) => devicesUnder(g.id))];
  }
  /** Picks all the devices in a folder, or unpicks them when they are all picked already. */
  function togglePickedFolder(g: Group) {
    const ids = devicesUnder(g.id).map((d) => d.id);
    const all = ids.length > 0 && ids.every((id) => picked.includes(id));
    picked = all ? picked.filter((id) => !ids.includes(id)) : [...new Set([...picked, ...ids])];
  }
  function folderPicked(g: Group): boolean | "some" {
    const ids = devicesUnder(g.id).map((d) => d.id);
    const n = ids.filter((id) => picked.includes(id)).length;
    return n === 0 ? false : n === ids.length ? true : "some";
  }
  function openMoveDialog() {
    const current = new Set(devices.filter((d) => picked.includes(d.id)).map((d) => d.group ?? ""));
    moveDialog = { target: current.size === 1 ? [...current][0] : "" };
  }
  async function setGroups(moves: { id: string; group: string | null }[]) {
    for (const m of moves) setTree(await invoke<DeviceTree>("move_device", m));
  }
  async function movePicked(event: SubmitEvent) {
    event.preventDefault();
    if (!moveDialog) return;
    const group = moveDialog.target || null;
    const moving = devices.filter((d) => picked.includes(d.id) && (d.group ?? null) !== group);
    const where = group ? (groups.find((g) => g.id === group)?.name ?? "") : "I miei dispositivi";
    try {
      await setGroups(moving.map((d) => ({ id: d.id, group })));
      if (moving.length > 0) {
        const what = moving.length === 1 ? `"${moving[0].name}" spostato` : `${moving.length} dispositivi spostati`;
        lastMove = { message: `${what} in ${where}`, previous: moving.map((d) => ({ id: d.id, group: d.group ?? null })) };
        clearTimeout(lastMoveTimer);
        lastMoveTimer = setTimeout(() => (lastMove = null), 10000);
      }
      moveDialog = null;
      picked = [];
    } catch (e) {
      addLog("error", String(e));
      moveDialog = null;
      await reloadDevices();
    }
  }
  /** Export of "my devices" (ADR 0057): with the saved passwords, protected by one. */
  let exportDialog = $state<{
    withPasswords: boolean;
    password: string;
    confirm: string;
    error: string;
    busy: boolean;
  } | null>(null);
  /** An encrypted file being imported, waiting for its password. */
  let importDialog = $state<{ content: string; password: string; error: string; busy: boolean } | null>(null);
  let importInput = $state<HTMLInputElement | null>(null);
  /** Outcome of the last export or import, shown above the list for a while. */
  let devicesMessage = $state<{ text: string; error: boolean } | null>(null);
  let devicesMessageTimer: ReturnType<typeof setTimeout> | undefined;
  const MIN_FILE_PASSWORD = 8;
  function showDevicesMessage(text: string, error = false) {
    devicesMessage = { text, error };
    clearTimeout(devicesMessageTimer);
    devicesMessageTimer = setTimeout(() => (devicesMessage = null), 15000);
  }
  function openExport() {
    exportDialog = { withPasswords: true, password: "", confirm: "", error: "", busy: false };
  }
  async function exportDevices(event: SubmitEvent) {
    event.preventDefault();
    if (!exportDialog || exportDialog.busy) return;
    const { withPasswords, password, confirm } = exportDialog;
    if (withPasswords && password.length < MIN_FILE_PASSWORD) {
      exportDialog.error = `La password deve avere almeno ${MIN_FILE_PASSWORD} caratteri.`;
      return;
    }
    if (withPasswords && password !== confirm) {
      exportDialog.error = "Le due password non coincidono.";
      return;
    }
    exportDialog.busy = true;
    try {
      const path = await invoke<string>("export_devices", { password: withPasswords ? password : null });
      exportDialog = null;
      showDevicesMessage(`Lista esportata in ${path}`);
      addLog("info", `lista dei dispositivi esportata in ${path}`);
    } catch (e) {
      if (exportDialog) {
        exportDialog.error = String(e);
        exportDialog.busy = false;
      }
    }
  }
  async function pickImportFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    if (file.size > 4 * 1024 * 1024) {
      showDevicesMessage("File troppo grande: non è una lista di dispositivi.", true);
      return;
    }
    try {
      const content = await file.text();
      if (await invoke<boolean>("devices_file_locked", { content })) {
        importDialog = { content, password: "", error: "", busy: false };
      } else {
        await importDevices(content, null);
      }
    } catch (e) {
      showDevicesMessage(String(e), true);
    }
  }
  type Imported = { devices: number; groups: number; tree: DeviceTree };
  async function importDevices(content: string, password: string | null) {
    const result = await invoke<Imported>("import_devices", { content, password });
    setTree(result.tree);
    const what = `${result.devices} ${result.devices === 1 ? "dispositivo" : "dispositivi"}`;
    const folders = result.groups > 0 ? ` e ${result.groups} ${result.groups === 1 ? "cartella" : "cartelle"}` : "";
    showDevicesMessage(`Importati ${what}${folders}.`);
    addLog("info", `importati ${what}${folders} nella lista dei dispositivi`);
  }
  async function importLocked(event: SubmitEvent) {
    event.preventDefault();
    if (!importDialog || importDialog.busy) return;
    importDialog.busy = true;
    try {
      await importDevices(importDialog.content, importDialog.password);
      importDialog = null;
    } catch (e) {
      if (importDialog) {
        importDialog.error = String(e);
        importDialog.busy = false;
      }
    }
  }
  async function undoMove() {
    if (!lastMove) return;
    const { previous } = lastMove;
    lastMove = null;
    clearTimeout(lastMoveTimer);
    try {
      await setGroups(previous);
    } catch (e) {
      addLog("error", String(e));
    }
  }
  /** This app's devices found on the local network (connect without the server). */
  type LanPeer = { id: string; host: string; alias: string | null };
  /** Online state of the saved devices (id digits → online); null while unknown. */
  let presence = $state<Record<string, boolean> | null>(null);
  const digits = (id: string) => id.replace(/\D/g, "");
  /** Online through the server or seen on this network; null when we cannot tell. */
  function deviceOnline(d: Device): boolean | null {
    if (lanPeers.some((p) => digits(p.id) === digits(d.id))) return true;
    return presence?.[digits(d.id)] ?? null;
  }
  // Opening the list asks again (the app also asks every 30 s on its own).
  $effect(() => {
    if (section === "devices" || section === "support") refreshPresence();
  });
  /** The home page's shortcuts: three saved devices, those online first. */
  const homeDevices = $derived(
    [...devices]
      .sort((a, b) => Number(deviceOnline(b) === true) - Number(deviceOnline(a) === true))
      .slice(0, 3),
  );
  function refreshPresence() {
    invoke<Record<string, boolean> | null>("device_presence").then((p) => {
      if (p !== null) presence = p;
    });
  }
  let lanPeers = $state<LanPeer[]>([]);
  /** Popup shown in the main window when a session ends (we controlled or were controlled). */
  let sessionEnded = $state<{
    session: number;
    controlled: boolean;
    peer: string;
    duration: number;
    reason: string;
  } | null>(null);
  /** The summary closes by itself (as OK) after this long. */
  const ENDED_CLOSE_SECS = 30;
  let endedLeft = $state(ENDED_CLOSE_SECS);
  $effect(() => {
    const shown = sessionEnded?.session;
    if (shown === undefined) return;
    endedLeft = ENDED_CLOSE_SECS;
    const timer = setInterval(() => {
      endedLeft--;
      if (endedLeft <= 0 && sessionEnded?.session === shown) acknowledgeEnded();
    }, 1000);
    return () => clearInterval(timer);
  });
  /** OK on the summary: nothing of that session may stay open (connection, viewer window). */
  function acknowledgeEnded() {
    if (sessionEnded) invoke("end_session", { session: sessionEnded.session });
    sessionEnded = null;
  }
  /** Partner of each session we viewed (from view_opened), for the end popup. */
  const viewedPeers = new Map<number, string>();
  /** Main window page chosen in the sidebar. */
  type Section = "support" | "devices" | "chat" | "activity" | "preferences";
  let section = $state<Section>("support");
  /** Stroke icons (24×24, lucide style) for the sidebar. */
  const icons: Record<Section, string> = {
    support: '<path d="M8 3 4 7l4 4"/><path d="M4 7h16"/><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/>',
    devices: '<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>',
    chat: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
    activity: '<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>',
    preferences:
      '<path d="M4 6h10"/><path d="M18 6h2"/><circle cx="16" cy="6" r="2"/><path d="M4 12h4"/><path d="M12 12h8"/><circle cx="10" cy="12" r="2"/><path d="M4 18h12"/><circle cx="18" cy="18" r="2"/>',
  };
  /** Renaming a LAN device (in-app dialog: the macOS web view has no window.prompt). */
  let renameDialog = $state<{ id: string; host: string; name: string; error: string } | null>(null);
  type UpdateInfo = { version: string; current: string; notes: string | null };
  type UpdateState =
    | { kind: "idle" }
    | { kind: "checking" }
    | { kind: "none" }
    | { kind: "available"; info: UpdateInfo }
    | { kind: "installing"; info: UpdateInfo; downloaded: number; total: number | null }
    | { kind: "error"; message: string };
  let appVersion = $state("");
  type Permissions = { required: boolean; screen: boolean; accessibility: boolean };
  let permissions = $state<Permissions | null>(null);
  type UacSupport = { supported: boolean; active: boolean; toggleable: boolean; outdated: boolean };
  let uac = $state<UacSupport | null>(null);
  let uacBusy = $state(false);
  /** Windows: reached remotely only, no monitor (a disconnected Remote Desktop session is
   * moved to the console by the service so it can be shared). */
  let remoteOnly = $state(false);
  /** What partners may do here besides seeing and controlling (sessions from now on). */
  let hostPermissions = $state({ clipboard: true, files: true });
  /** "Solo da remoto" exists on Windows (session moved to the screen, ADR 0059) and on the Mac. */
  const isMac = navigator.userAgent.includes("Mac OS X");

  function savedTheme(): Theme {
    try {
      const saved = localStorage.getItem("theme");
      return saved === "light" || saved === "dark" ? saved : "system";
    } catch {
      return "system";
    }
  }

  /** `data-theme` on <html> overrides the system's appearance (see the :root rules). */
  function applyTheme(value: Theme) {
    if (value === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = value;
  }
  applyTheme(savedTheme());
  /** Asking whether this PC is reached remotely only (after unattended access is turned on). */
  let remoteOnlyAsk = $state(false);
  /** Windows: the virtual monitor for PCs without one (ADR 0049). */
  type VirtualMonitor = {
    supported: boolean;
    installed: boolean;
    version: string | null;
    bundled: string | null;
    message: string | null;
  };
  let virtualMonitor = $state<VirtualMonitor | null>(null);
  let virtualMonitorBusy = $state(false);
  /** "Scarica i log": busy while the zip is written, then where it went (or the error). */
  let logsExport = $state<{ busy: boolean; message: string; error: boolean }>({
    busy: false,
    message: "",
    error: false,
  });
  let virtualMonitorError = $state("");
  /** macOS, remote only: the virtual display's possible sizes and the one fixed (ADR 0065). */
  type VirtualSize = { sizes: [number, number][]; fixed: [number, number] | null };
  let virtualSize = $state<VirtualSize | null>(null);
  /** Light, dark or as the system (remembered on this computer only). */
  type Theme = "system" | "light" | "dark";
  let theme = $state<Theme>(savedTheme());
  let update = $state<UpdateState>({ kind: "idle" });
  let autostart = $state(false);
  /** Our user name as partners see it, and the text being edited. */
  let displayName = $state<string | null>(null);
  let nameDraft = $state("");
  let usePersonal = $state(false);
  let saveAs = $state("");
  /** Personal password dialog (set / change). */
  let personalDialog = $state<{ first: string; second: string; error: string } | null>(null);
  /** Menu (⋯) of the unattended access row: change password, turn off. */
  let unattendedMenu = $state(false);
  let partnerId = $state("");
  let partnerPassword = $state("");
  let formError = $state("");
  let busy = $state(false);
  let logs = $state<LogLine[]>([]);
  let approvals = $state<Approval[]>([]);
  let now = $state(Date.now());
  let copied = $state("");
  let view = $state<View | null>(null);
  /** This window shows a remote screen (`view-<session>`) instead of the main app. */
  const viewSession = (() => {
    const label = getCurrentWindow().label;
    return label.startsWith("view-") ? Number(label.slice(5)) : null;
  })();
  /** This window is the file manager of a session we view (`files-<session>`). */
  const filesSession = (() => {
    const label = getCurrentWindow().label;
    return label.startsWith("files-") ? Number(label.slice(6)) : null;
  })();
  /** This window is the floating chat of a shared session (`chat-<session>`). */
  const chatPopupSession = (() => {
    const label = getCurrentWindow().label;
    return label.startsWith("chat-") ? Number(label.slice(5)) : null;
  })();
  let popupChat = $state<ChatMsg[]>([]);
  let popupPeer = $state("");
  let hosting = $state<Hosting[]>([]);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let remote: RemoteVideo | null = null;

  // ---- Picture size in the viewer ----------------------------------------------------
  /**
   * "fit": fills the window (enlarged with the bicubic shader); "whole": the largest whole
   * multiple of the remote pixels that fits, every pixel an exact square; "native": one
   * remote pixel per screen pixel, with scroll bars when larger than the window; "remote": the
   * partner sets its screen to the window's size (only computers used remotely, ADR 0061),
   * otherwise shown as "fit".
   */
  type Zoom = "fit" | "whole" | "native" | "remote";
  const zoomModes: Zoom[] = ["fit", "whole", "native", "remote"];
  let zoom = $state<Zoom>(
    (() => {
      try {
        const saved = localStorage.getItem("viewZoom") as Zoom | null;
        return saved && zoomModes.includes(saved) ? saved : "fit";
      } catch {
        return "fit";
      }
    })(),
  );
  const shownZoom = $derived<Zoom>(zoom === "remote" && !view?.resizable ? "fit" : zoom);
  let screenEl = $state<HTMLDivElement | null>(null);
  let screenBox = $state({ width: 0, height: 0 });
  let pixelRatio = $state(window.devicePixelRatio || 1);
  /** Size of the frames being painted (0 before the first one). */
  let frameSize = $state({ width: 0, height: 0 });
  /** Largest whole multiple of the frame that fits the window (at least 1). */
  const wholeMultiple = $derived(
    frameSize.width && screenBox.width
      ? Math.max(
          1,
          Math.floor(
            Math.min(
              (screenBox.width * pixelRatio) / frameSize.width,
              (screenBox.height * pixelRatio) / frameSize.height,
            ),
          ),
        )
      : 1,
  );
  /** Size of the picture on screen, in CSS and in device pixels. */
  const picture = $derived.by(() => {
    const { width, height } = frameSize;
    if (!width || !height || !screenBox.width || !screenBox.height) return null;
    let device: { width: number; height: number };
    if (shownZoom === "fit" || shownZoom === "remote") {
      const scale = Math.min(screenBox.width / width, screenBox.height / height) * pixelRatio;
      device = { width: Math.floor(width * scale), height: Math.floor(height * scale) };
    } else {
      const k = shownZoom === "whole" ? wholeMultiple : 1;
      device = { width: width * k, height: height * k };
    }
    return {
      ...device,
      cssWidth: device.width / pixelRatio,
      cssHeight: device.height / pixelRatio,
      filter: shownZoom === "whole" || shownZoom === "native" ? ("nearest" as const) : ("bicubic" as const),
    };
  });

  $effect(() => {
    if (picture) remote?.renderer.setOutput(picture.width, picture.height, picture.filter);
  });

  $effect(() => {
    const el = screenEl;
    if (!el) return;
    // Client size: without the scroll bars of a 1:1 picture larger than the window.
    const observer = new ResizeObserver(() => {
      screenBox = { width: el.clientWidth, height: el.clientHeight };
    });
    observer.observe(el);
    return () => observer.disconnect();
  });

  $effect(() => {
    // Moving the window to a screen with another scale factor.
    const query = matchMedia(`(resolution: ${pixelRatio}dppx)`);
    const changed = () => (pixelRatio = window.devicePixelRatio || 1);
    query.addEventListener("change", changed);
    return () => query.removeEventListener("change", changed);
  });

  /** Last size asked of the partner ("remote" mode), so it is asked again only when it changes. */
  let askedResolution: string | null = null;
  $effect(() => {
    const v = view;
    if (!v || v.ended) return;
    const session = v.session;
    const scale = pixelRatio;
    const width = Math.round(screenBox.width * scale);
    const height = Math.round(screenBox.height * scale);
    // Asked again with HD (the partner's limit) or another monitor.
    const key = `${session}:${width}x${height}@${scale}:${v.hd}:${v.currentDisplay}`;
    if (shownZoom !== "remote") {
      if (askedResolution !== null) {
        askedResolution = null;
        invoke("match_resolution", { session, width: 0, height: 0, scale });
      }
      return;
    }
    if (!width || !height || key === askedResolution) return;
    // Resizing the window: ask once it stops, every change costs the partner a mode switch.
    const timer = setTimeout(() => {
      askedResolution = key;
      invoke("match_resolution", { session, width, height, scale });
    }, 800);
    return () => clearTimeout(timer);
  });

  function setZoom(value: Zoom) {
    zoom = value;
    try {
      localStorage.setItem("viewZoom", value);
    } catch {
      // not saved: the default comes back next time
    }
  }

  /** "quick": share-only Quick access build (no installation); unknown until asked. */
  let mode = $state<"quick" | "full" | null>(null);
  const quick = $derived(mode === "quick");
  const appName = $derived(quick ? "Kronos Quick Support" : "Kronos Remote Support");
  const navItems = $derived<{ id: Section; label: string }[]>([
    { id: "support", label: "Supporto remoto" },
    ...(quick ? [] : [{ id: "devices" as const, label: "Dispositivi" }]),
    { id: "chat", label: "Chat" },
    { id: "activity", label: "Attività" },
  ]);

  /** 45 → "45 s", 750 → "12 min 30 s", 3900 → "1 h 05 min". */
  function formatDuration(seconds: number) {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    if (h > 0) return `${h} h ${String(m).padStart(2, "0")} min`;
    if (m > 0) return `${m} min ${String(s).padStart(2, "0")} s`;
    return `${s} s`;
  }

  const decimal = (value: number, digits: number) =>
    value.toLocaleString("it-IT", { maximumFractionDigits: digits });
  /** kbit/s → "850 kbit/s" / "3,2 Mbit/s". */
  const rate = (kbps: number) =>
    kbps >= 1000 ? `${decimal(kbps / 1000, 1)} Mbit/s` : `${Math.round(kbps)} kbit/s`;
  const pct = (cpu: number) => `${decimal(cpu, cpu < 10 ? 1 : 0)}%`;
  /** Toolbar highlight: yellow when a value deserves attention, red when it hurts. */
  type Level = "" | "caution" | "critical";
  const level = (value: number | null | undefined, caution: number, critical: number): Level =>
    value == null ? "" : value >= critical ? "critical" : value >= caution ? "caution" : "";
  const worst = (...levels: Level[]): Level =>
    levels.includes("critical") ? "critical" : levels.includes("caution") ? "caution" : "";
  const RESYNC_WINDOW_MS = 60_000;

  /** Overall state for the dot next to the connection type: the worst of the statistics. */
  function connectionLevel(v: View): { level: "ok" | Level; reasons: string[] } {
    const checks: [Level, string][] = [
      [level(v.rtt, 150, 300), `Latenza ${v.rtt?.toFixed(0)} ms`],
      [level(v.local?.cpu, 50, 80), `CPU tua ${v.local?.cpu.toFixed(0)} %`],
      [level(v.partner?.cpu, 50, 80), `CPU partner ${v.partner?.cpu.toFixed(0)} %`],
      [level(v.local?.ram, 800, 1500), `Memoria tua ${v.local?.ram} MB`],
      [level(v.partner?.ram, 800, 1500), `Memoria partner ${v.partner?.ram} MB`],
      [v.limited ? (v.limited.kbps < 450 ? "critical" : "caution") : "", "Qualità ridotta per la banda"],
      [level(recentResyncs(v), 2, 5), `${recentResyncs(v)} resync nell'ultimo minuto`],
    ];
    const found = checks.filter(([l]) => l !== "");
    const overall = worst(...found.map(([l]) => l));
    return { level: overall || "ok", reasons: found.map(([l, why]) => `${l === "critical" ? "Critico" : "Attenzione"}: ${why}`) };
  }
  // Reads the reactive `now`, so the count drops as resyncs get older than a minute.
  const recentResyncs = (v: View) => v.resyncs.filter((t) => now - t < RESYNC_WINDOW_MS).length;
  const mb = (ram: number) => (ram >= 1000 ? `${decimal(ram / 1000, 1)} GB` : `${ram} MB`);
  /** Right-aligns a value in a fixed number of characters (monospaced toolbar stats). */
  const pad = (text: string | number, width: number) => String(text).padStart(width);

  const pathLabel: Record<Path, string> = {
    lan: "Diretta (rete locale)",
    direct: "Diretta (Internet)",
    relay: "Relay TURN",
  };
  const pathHint: Record<Path, string> = {
    lan: "Collegamento diretto tra i due computer sulla stessa rete",
    direct: "Collegamento diretto via Internet (NAT attraversato)",
    relay: "Il traffico passa dal server relay: la rete non permette il collegamento diretto. Resta cifrato end-to-end.",
  };

  const linkLabel: Record<Link, string> = {
    connecting: "Connessione al server…",
    online: "Pronto",
    offline: "Offline",
  };

  const approval = $derived(approvals[0] ?? null);
  const secondsLeft = $derived(
    approval ? Math.max(0, Math.ceil((approval.deadline - now) / 1000)) : 0,
  );
  // Clock for the approval countdown (smooth bar) and the viewer's resyncs of the last
  // minute; nothing ticks when neither is shown.
  const clockStep = $derived(approval ? 250 : view && !view.ended ? 1000 : 0);
  $effect(() => {
    if (!clockStep) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), clockStep);
    return () => clearInterval(timer);
  });

  function addLog(level: string, message: string) {
    const time = new Date().toLocaleTimeString("it-IT");
    logs = [{ time, level, message }, ...logs].slice(0, 80);
    if (level === "error" && busy) {
      busy = false;
    }
  }

  function handle(event: UiEvent) {
    switch (event.kind) {
      case "presence":
        presence = event.devices;
        break;
      case "status": {
        const { kind, ...rest } = event;
        status = rest;
        break;
      }
      case "log":
        addLog(event.level, event.message);
        break;
      case "view_opened":
        busy = false;
        // This window waited to connect again: the new session has its own window.
        if (view?.reconnecting && view.peer === event.peer && view.session !== event.session) {
          view.reconnecting = false;
          getCurrentWindow().close();
          break;
        }
        viewedPeers.set(event.session, event.peer);
        if (view?.session === event.session) {
          view.peer = event.peer;
          view.peerName = event.peer_name;
        }
        break;
      case "view_size":
        if (view?.session === event.session) {
          // A new size means a new stream (other monitor, or resolution adapted to the
          // bandwidth): restart the decoder, unless it already shows the new size.
          const painted = frameSize.width === event.width && frameSize.height === event.height;
          if (view.width !== null && (view.width !== event.width || view.height !== event.height) && !painted) {
            remote?.restart();
          }
          view.width = event.width;
          view.height = event.height;
          view.hardware = event.hardware;
          view.noScreen = null;
        }
        break;
      case "view_no_screen":
        if (view?.session === event.session) view.noScreen = event.message;
        break;
      case "view_displays":
        if (view?.session === event.session) {
          view.displays = event.displays;
          view.currentDisplay = event.current;
        }
        break;
      case "view_control_unavailable":
        if (view?.session === event.session) {
          view.control = false;
          view.controlError = event.message;
        }
        break;
      case "view_stats":
        if (view?.session === event.session) {
          view.net = { rx: event.rx_kbps, tx: event.tx_kbps };
          if (event.cpu !== null && event.ram_mb !== null)
            view.local = { cpu: event.cpu, ram: event.ram_mb };
        }
        break;
      case "view_partner_usage":
        if (view?.session === event.session) view.partner = { cpu: event.cpu, ram: event.ram_mb };
        break;
      case "view_quality":
        if (view?.session === event.session)
          view.limited = event.limited ? { kbps: event.kbps, fps: event.fps } : null;
        break;
      case "view_resync":
        if (view?.session === event.session && !view.resyncStarted) {
          view.resyncStarted = true;
        } else if (view?.session === event.session) {
          const at = Date.now();
          view.resyncs = [...view.resyncs.filter((t) => at - t < RESYNC_WINDOW_MS), at];
        }
        break;
      case "view_rtt":
        if (view?.session === event.session) view.rtt = event.ms;
        break;
      case "view_closed":
        if (view?.session === event.session) {
          endVideo(event.reason);
          // The partner quit to install an update: nothing more to see here (the summary in
          // the main window says why).
          if (event.updating) getCurrentWindow().close();
          // Connecting again by ourselves: say so until the new window opens.
          if (event.reconnect && view) view.reconnecting = true;
        }
        if (viewSession === null && event.reconnect) {
          viewedPeers.delete(event.session);
        } else if (viewSession === null) {
          // Main window: summary popup for the session we controlled.
          sessionEnded = {
            session: event.session,
            controlled: false,
            peer: viewedPeers.get(event.session) ?? "",
            duration: event.duration_secs,
            reason: event.reason,
          };
          viewedPeers.delete(event.session);
        }
        break;
      case "reconnect_failed":
        if (view?.reconnecting && view.peer === event.peer) {
          view.reconnecting = false;
          view.ended = `${view.ended}; il computer remoto non è tornato online`;
        }
        break;
      case "view_clipboard":
        if (view?.session === event.session) view.clipboard = event.enabled;
        break;
      case "view_silent":
        if (view?.session === event.session) view.silent = event.silent;
        break;
      case "view_input_blocked":
        if (view?.session === event.session) view.inputBlocked = event.blocked;
        break;
      case "view_resolution_adjustable":
        if (view?.session === event.session) view.resizable = true;
        break;
      case "hosting_input_blocked":
        hosting = hosting.map((h) => (h.session === event.session ? { ...h, blocked: event.blocked } : h));
        break;
      case "view_audio":
        if (view?.session === event.session) view.audio = event.enabled;
        break;
      case "view_notice":
        if (view?.session === event.session) showNotice(event.message);
        break;
      case "view_chat":
        if (view?.session === event.session) receiveViewChat(view, event.text);
        break;
      case "hosting_started":
        hosting = [
          ...hosting,
          { session: event.session, peer: event.peer, path: null, chat: [], showChat: false, unread: 0, blocked: false, transfer: null },
        ];
        break;
      case "host_chat_sent": {
        // Written in another window: show it here too.
        if (event.window === getCurrentWindow().label) break;
        if (chatPopupSession === event.session) {
          popupChat.push({ mine: true, text: event.text, time: nowTime() });
        }
        const h = hosting.find((x) => x.session === event.session);
        if (h) h.chat.push({ mine: true, text: event.text, time: nowTime() });
        break;
      }
      case "transfer": {
        const t = event.finished
          ? null
          : { sending: event.sending, done: event.done, total: event.total, rate: event.bytes_per_sec };
        if (view?.session === event.session) view.transfer = t;
        hosting = hosting.map((h) => (h.session === event.session ? { ...h, transfer: t } : h));
        break;
      }
      case "host_chat": {
        if (chatPopupSession === event.session) {
          popupPeer = event.peer;
          popupChat.push({ mine: false, text: event.text, time: nowTime() });
          scrollPopup();
        }
        const h = hosting.find((x) => x.session === event.session);
        if (h) receiveChat(h, event.text);
        break;
      }
      case "connection_path":
        if (view?.session === event.session) view.path = event.path;
        hosting = hosting.map((h) => (h.session === event.session ? { ...h, path: event.path } : h));
        break;
      case "hosting_ended":
        hosting = hosting.filter((h) => h.session !== event.session);
        sessionEnded = {
          session: event.session,
          controlled: true,
          peer: event.peer,
          duration: event.duration_secs,
          reason: "",
        };
        break;
      case "approval_request":
        approvals = [
          ...approvals,
          {
            session: event.session,
            peer: event.peer,
            deadline: Date.now() + event.timeout_secs * 1000,
            total: event.timeout_secs,
          },
        ];
        break;
      case "approval_closed":
        approvals = approvals.filter((a) => a.session !== event.session);
        break;
    }
  }

  onMount(() => {
    // The file manager window listens on its own.
    if (filesSession !== null) return;
    const unlisten = listen<UiEvent>("service", (e) => handle(e.payload));
    if (chatPopupSession !== null) {
      invoke<{ mine: boolean; text: string }[]>("host_chat_history", { session: chatPopupSession }).then((lines) => {
        // The history already holds whatever arrived while this window was loading.
        popupChat = lines.map((l) => ({ ...l, time: "" }));
        scrollPopup();
      });
      getCurrentWindow()
        .title()
        .then((t) => (popupPeer = t.replace(/^Chat con /, "")));
      return () => {
        unlisten.then((f) => f());
      };
    }
    if (viewSession !== null) {
      invoke<boolean>("audio_supported").then((s) => (audioSupported = s));
      openView(viewSession);
      return () => {
        unlisten.then((f) => f());
      };
    }
    invoke<"quick" | "full">("app_mode").then((m) => (mode = m));
    invoke<Status>("get_status").then((s) => (status = s));
    reloadDevices();
    refreshPresence();
    invoke<boolean>("get_autostart").then((a) => (autostart = a));
    invoke<boolean>("remote_only").then((r) => (remoteOnly = r));
    // The theme chosen in another window of the app (viewer, chat) applies here too.
    window.addEventListener("storage", (e) => {
      if (e.key === "theme" || e.key === null) {
        theme = savedTheme();
        applyTheme(theme);
      }
    });
    invoke<{ clipboard: boolean; files: boolean }>("host_permissions").then((p) => (hostPermissions = p));
    invoke<string | null>("display_name").then((n) => {
      displayName = n;
      nameDraft = n ?? "";
    });
    invoke<string>("app_version").then((v) => (appVersion = v));
    const refreshPermissions = () => {
      invoke<Permissions>("get_permissions").then((p) => (permissions = p));
      // UAC state can change after the startup prompt (service installed/removed).
      invoke<UacSupport>("uac_support").then((u) => (uac = u));
      invoke<LanPeer[]>("lan_peers").then((p) => (lanPeers = p));
    };
    refreshPermissions();
    // Picks up changes made in System Settings while the app is open; not while the window
    // is hidden (in the tray), and at once when it shows again.
    const permissionTimer = setInterval(() => document.hidden || refreshPermissions(), 3000);
    const onVisibility = () => document.hidden || refreshPermissions();
    document.addEventListener("visibilitychange", onVisibility);
    const unlistenProgress = listen<{ downloaded: number; total: number | null }>(
      "update_progress",
      (e) => {
        if (update.kind === "installing") {
          update.downloaded = e.payload.downloaded;
          update.total = e.payload.total;
        }
      },
    );
    // Quiet check shortly after start; the button shows if something is available.
    const firstCheck = setTimeout(checkUpdate, 4000);
    // Then every few hours, for apps left open for days (e.g. unattended access).
    const updateTimer = setInterval(() => {
      if (update.kind !== "available") checkUpdate();
    }, UPDATE_CHECK_MS);
    return () => {
      unlisten.then((f) => f());
      unlistenProgress.then((f) => f());
      clearTimeout(firstCheck);
      clearInterval(updateTimer);
      clearInterval(permissionTimer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  /** Keeps only digits and groups them as "123 456 789". */
  function formatId(value: string) {
    const digits = value.replace(/\D/g, "").slice(0, 9);
    return digits.replace(/(\d{3})(?=\d)/g, "$1 ");
  }

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    formError = "";
    try {
      // False: already connected to that device, its window was brought forward.
      busy = await invoke<boolean>("connect_to", {
        target: partnerId,
        password: partnerPassword,
        personal: usePersonal,
        saveAs: usePersonal && saveAs.trim() ? saveAs.trim() : null,
      });
      if (usePersonal && saveAs.trim()) {
        await reloadDevices();
        saveAs = "";
        refreshPresence();
      }
    } catch (e) {
      formError = String(e);
    }
  }

  /** Splits "1.2.3" into numbers; anything after the version (" — date") is ignored. */
  function versionParts(v: string): number[] {
    return (v.match(/^v?(\d+(?:\.\d+)*)/)?.[1] ?? "0").split(".").map(Number);
  }

  function newerThan(a: string, b: string): boolean {
    const x = versionParts(a);
    const y = versionParts(b);
    for (let i = 0; i < Math.max(x.length, y.length); i++) {
      if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
    }
    return false;
  }

  /**
   * Release notes carry the latest CHANGELOG.md sections ("## 1.2.1 — date"): keeps the
   * versions newer than the installed one. Notes without sections are shown as they are.
   */
  function changesSince(notes: string, current: string): { title: string; body: string }[] {
    const sections = notes
      .split(/^## /m)
      .slice(1)
      .map((chunk) => {
        const end = chunk.indexOf("\n");
        const title = (end < 0 ? chunk : chunk.slice(0, end)).trim();
        return { title, body: end < 0 ? "" : chunk.slice(end + 1).trim() };
      })
      .filter((s) => /^\d/.test(s.title));
    if (sections.length === 0) return [{ title: "", body: notes.trim() }];
    return sections.filter((s) => newerThan(s.title, current));
  }

  /** Result shown in the Aggiornamenti page; the sidebar marks an available version. */
  const UPDATE_CHECK_MS = 4 * 60 * 60 * 1000;

  async function checkUpdate() {
    if (update.kind === "checking" || update.kind === "installing") return;
    update = { kind: "checking" };
    try {
      const info = await invoke<UpdateInfo | null>("check_update");
      update = info ? { kind: "available", info } : { kind: "none" };
    } catch (e) {
      update = { kind: "error", message: String(e) };
    }
  }

  type PrefsTab = "general" | "connections" | "screen" | "updates" | "support";
  let prefsTab = $state<PrefsTab>("general");
  const prefsTabs = $derived<{ id: PrefsTab; label: string }[]>([
    { id: "general", label: "Generale" },
    { id: "connections", label: "Connessioni" },
    ...(uac?.supported || isMac || virtualMonitor?.supported ? [{ id: "screen" as const, label: "Schermo" }] : []),
    { id: "updates", label: "Aggiornamenti" },
    { id: "support", label: "Assistenza" },
  ]);
  /** "Full HD", "16:10"… under a size in the preferences. */
  function sizeName([w, h]: [number, number]) {
    const names: Record<string, string> = { "1280x720": "HD", "1920x1080": "Full HD", "2560x1440": "QHD" };
    return names[`${w}x${h}`] ?? (Math.abs(w / h - 1.6) < 0.01 ? "16:10" : "16:9");
  }

  /** Opening the section checks for updates again (an update already found stays shown). */
  function openPreferences(tab?: PrefsTab) {
    section = "preferences";
    if (tab) prefsTab = tab;
    else if (update.kind === "available") prefsTab = "updates";
    if (update.kind !== "available") checkUpdate();
    invoke<VirtualMonitor>("virtual_monitor_status").then((v) => (virtualMonitor = v));
    invoke<VirtualSize>("virtual_size").then((v) => (virtualSize = v));
  }

  /** "Automatica" (null) or one of the sizes, as "1920x1080". */
  async function setVirtualSize(value: string) {
    const size = value ? (value.split("x").map(Number) as [number, number]) : null;
    try {
      virtualSize = await invoke<VirtualSize>("set_virtual_size", { size });
      addLog("info", size
        ? `schermo virtuale a ${size[0]}×${size[1]} dalla prossima connessione`
        : "schermo virtuale alla dimensione della finestra di chi controlla");
    } catch (e) {
      addLog("error", String(e));
    }
  }

  function setTheme(value: Theme) {
    theme = value;
    applyTheme(value);
    try {
      if (value === "system") localStorage.removeItem("theme");
      else localStorage.setItem("theme", value);
    } catch {
      // Not remembered: this run only.
    }
  }

  /** Installs/updates (`install`) or removes the virtual monitor; Windows asks for admin rights. */
  async function exportLogs() {
    if (logsExport.busy) return;
    logsExport = { busy: true, message: "", error: false };
    try {
      const path = await invoke<string>("export_logs");
      logsExport = { busy: false, message: `Log salvati in ${path}`, error: false };
      addLog("info", `log salvati in ${path}`);
    } catch (e) {
      logsExport = { busy: false, message: String(e), error: true };
    }
  }
  async function setVirtualMonitor(install: boolean) {
    if (!install) {
      const ok = await askConfirm(
        "Rimuovere il monitor virtuale? Senza monitor collegato, questo PC non avrà più uno schermo da condividere.",
        "Rimuovi",
      );
      if (!ok) return;
    }
    virtualMonitorBusy = true;
    virtualMonitorError = "";
    try {
      virtualMonitor = await invoke<VirtualMonitor>("set_virtual_monitor", { install });
    } catch (e) {
      virtualMonitorError = String(e);
    } finally {
      virtualMonitorBusy = false;
    }
  }

  async function installUpdate() {
    if (update.kind !== "available") return;
    update = { kind: "installing", info: update.info, downloaded: 0, total: null };
    try {
      await invoke("install_update"); // the app restarts when done
    } catch (e) {
      update = { kind: "error", message: String(e) };
    }
  }

  /** Never leave the form stuck: the backend always answers within ~70 s, this is a backstop. */
  let busyGuard: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    clearTimeout(busyGuard);
    if (busy) busyGuard = setTimeout(() => (busy = false), 90_000);
  });

  async function cancelConnect() {
    await invoke("cancel_connect");
    busy = false;
  }

  async function connectDevice(d: Device) {
    formError = "";
    try {
      busy = await invoke<boolean>("connect_device", { id: d.id });
    } catch (e) {
      addLog("error", String(e));
    }
  }

  /** In-app confirmation: the web view on macOS has no `window.confirm` (it returns false). */
  let confirmDialog = $state<{
    message: string;
    action: string;
    resolve: (ok: boolean) => void;
  } | null>(null);
  function askConfirm(message: string, action: string): Promise<boolean> {
    return new Promise((resolve) => (confirmDialog = { message, action, resolve }));
  }
  function answerConfirm(ok: boolean) {
    confirmDialog?.resolve(ok);
    confirmDialog = null;
  }

  async function toggleUac() {
    if (!uac || uacBusy) return;
    uacBusy = true;
    try {
      uac = await invoke<UacSupport>("set_uac_support", { enable: !uac.active });
      addLog("info", uac.active ? "controllo finestre amministratore attivo" : "controllo finestre amministratore disattivato");
    } catch (e) {
      addLog("error", String(e));
    } finally {
      uacBusy = false;
    }
  }
  /** Reinstalls the service with this version (it keeps its own copy of the app). */
  async function updateUac() {
    if (!uac || uacBusy) return;
    uacBusy = true;
    try {
      uac = await invoke<UacSupport>("set_uac_support", { enable: true });
      addLog("info", "servizio Windows aggiornato");
    } catch (e) {
      addLog("error", String(e));
    } finally {
      uacBusy = false;
    }
  }
  async function relaunchAdmin() {
    try {
      const ok = await invoke<boolean>("relaunch_elevated");
      if (!ok) addLog("warn", "permesso di amministratore negato: continuo con i permessi utente");
    } catch (e) {
      addLog("error", String(e));
    }
  }

  async function saveLanName(event: SubmitEvent) {
    event.preventDefault();
    if (!renameDialog) return;
    try {
      lanPeers = await invoke<LanPeer[]>("set_lan_name", {
        id: renameDialog.id,
        name: renameDialog.name,
      });
      renameDialog = null;
    } catch (e) {
      if (renameDialog) renameDialog.error = String(e);
    }
  }

  async function removeDevice(d: Device) {
    if (!(await askConfirm(`Rimuovere "${d.name}" da I miei dispositivi?`, "Rimuovi"))) return;
    await invoke("remove_device", { id: d.id });
    await reloadDevices();
  }

  async function savePersonalPassword(event: SubmitEvent) {
    event.preventDefault();
    if (!personalDialog) return;
    if (personalDialog.first !== personalDialog.second) {
      personalDialog.error = "le due password non coincidono";
      return;
    }
    try {
      const wasUnattended = status.unattended;
      status = await invoke<Status>("set_personal_password", { password: personalDialog.first });
      personalDialog = null;
      addLog("info", "password personale impostata: accesso non presidiato attivo");
      if (!wasUnattended && (uac?.supported || isMac) && !quick) remoteOnlyAsk = true;
    } catch (e) {
      if (personalDialog) personalDialog.error = String(e);
    }
  }

  async function disablePersonal() {
    const message =
      "Disattivare l'accesso non presidiato? Servirà di nuovo la conferma per ogni connessione.";
    if (!(await askConfirm(message, "Disattiva"))) return;
    status = await invoke<Status>("set_personal_password", { password: null });
    addLog("info", "accesso non presidiato disattivato");
  }

  let editingName = $state(false);

  async function saveName() {
    if (nameDraft.trim() !== (displayName ?? "")) {
      displayName = await invoke<string | null>("set_display_name", { name: nameDraft });
    }
    nameDraft = displayName ?? "";
    editingName = false;
  }

  function cancelName() {
    nameDraft = displayName ?? "";
    editingName = false;
  }

  async function setRemoteOnly(enabled: boolean) {
    remoteOnlyAsk = false;
    try {
      remoteOnly = await invoke<boolean>("set_remote_only", { enabled });
      addLog("info", remoteOnly
        ? isMac
          ? "computer usato solo da remoto: chi controlla può adattare la risoluzione alla sua finestra"
          : "computer usato solo da remoto: la sessione di Desktop remoto chiusa resta condivisibile e chi controlla può adattare la risoluzione"
        : "computer con monitor: risoluzione e sessione non vengono cambiate da remoto");
    } catch (e) {
      addLog("error", String(e));
    }
  }

  async function setHostPermission(kind: "clipboard" | "files", enabled: boolean) {
    try {
      hostPermissions = await invoke("set_host_permissions", {
        allowed: { ...hostPermissions, [kind]: enabled },
      });
      const what = kind === "clipboard" ? "appunti condivisi" : "trasferimento file";
      addLog("info", `${what} ${enabled ? "consentito" : "non consentito"} a chi si collega (dalle prossime sessioni)`);
    } catch (e) {
      addLog("error", String(e));
    }
  }

  async function toggleAutostart() {
    try {
      autostart = await invoke<boolean>("set_autostart", { enabled: !autostart });
    } catch (e) {
      addLog("error", String(e));
    }
  }

  async function respond(accept: boolean) {
    if (!approval) return;
    const { session } = approval;
    approvals = approvals.filter((a) => a.session !== session);
    await invoke("respond_approval", { session, accept });
  }

  async function newPassword() {
    status = await invoke<Status>("new_password");
  }

  async function copy(text: string | null, what: string) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text.replace(/\s/g, ""));
      copied = what;
      setTimeout(() => (copied = ""), 1500);
    } catch {
      addLog("warn", "copia negli appunti non riuscita");
    }
  }

  function openView(session: number) {
    // Closed while waiting to connect again: no more attempts.
    getCurrentWindow().onCloseRequested(() => {
      if (view?.reconnecting) invoke("cancel_connect");
    });
    view = {
      session,
      peer: "",
      width: null,
      height: null,
      hardware: null,
      rtt: null,
      fps: null,
      waiting: true,
      error: null,
      noScreen: null,
      control: true,
      controlError: null,
      displays: [],
      currentDisplay: null,
      resyncs: [],
      resyncStarted: false,
      hd: true,
      audio: false,
      inputBlocked: false,
      resizable: false,
      peerName: null,
      transfer: null,
      path: null,
      limited: null,
      net: null,
      local: null,
      partner: null,
      clipboard: null,
      notice: null,
      ended: null,
      reconnecting: false,
      silent: false,
      chat: [],
      showChat: chatPinned,
      unread: 0,
      chatUsed: chatPinned,
      chatDismissed: false,
      chatPeek: null,
    };
    startVideo(session);
  }

  const nowTime = () => new Date().toLocaleTimeString("it-IT", { hour: "2-digit", minute: "2-digit" });
  function receiveChat(target: { chat: ChatMsg[]; showChat: boolean; unread: number }, text: string) {
    target.chat.push({ mine: false, text, time: nowTime() });
    if (!target.showChat) target.unread += 1;
  }
  let chatDraft = $state("");
  async function sendChat(target: { session: number; chat: ChatMsg[] }, event: SubmitEvent) {
    event.preventDefault();
    const text = chatDraft.trim();
    if (!text) return;
    chatDraft = "";
    target.chat.push({ mine: true, text, time: nowTime() });
    try {
      await invoke("send_chat", { session: target.session, text });
    } catch (e) {
      addLog("error", String(e));
    }
  }
  function toggleChat(target: { showChat: boolean; unread: number }) {
    target.showChat = !target.showChat;
    if (target.showChat) target.unread = 0;
    chatDraft = "";
  }

  let popupLog = $state<HTMLDivElement | null>(null);
  function scrollPopup() {
    tick().then(() => popupLog?.scrollTo({ top: popupLog.scrollHeight }));
  }
  let popupDraft = $state("");
  async function sendPopupChat(event: SubmitEvent) {
    event.preventDefault();
    const text = popupDraft.trim();
    if (!text || chatPopupSession === null) return;
    popupDraft = "";
    popupChat.push({ mine: true, text, time: nowTime() });
    scrollPopup();
    await invoke("send_chat", { session: chatPopupSession, text });
  }

  function cancelTransfer(session: number) {
    invoke("cancel_transfer", { session });
  }

  /** "12,3 MB di 40,0 MB · 3,1 MB/s · 31 %" */
  function transferText(t: Transfer) {
    const mbs = (b: number) => `${decimal(b / 1_000_000, 1)} MB`;
    const pctDone = t.total > 0 ? Math.floor((t.done / t.total) * 100) : 0;
    return `${t.sending ? "Invio" : "Ricezione"} file · ${pctDone} % · ${mbs(t.done)} di ${mbs(t.total)} · ${mbs(t.rate)}/s`;
  }

  /** Hosted session whose chat the Chat page shows (null: the first one). */
  let chatSession = $state<number | null>(null);
  const activeChat = $derived(hosting.find((h) => h.session === chatSession) ?? hosting[0] ?? null);
  const hostUnread = $derived(hosting.reduce((n, h) => n + h.unread, 0));
  // A hosted chat counts as open (no unread) while the Chat page shows it.
  $effect(() => {
    for (const h of hosting) {
      h.showChat = section === "chat" && h.session === activeChat?.session;
      if (h.showChat) h.unread = 0;
    }
  });
  function openHostChat(session: number) {
    chatSession = session;
    section = "chat";
    chatDraft = "";
  }

  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  function showNotice(message: string) {
    if (!view) return;
    view.notice = message;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => view && (view.notice = null), 6000);
  }

  async function startVideo(session: number) {
    await tick(); // the viewer canvas is rendered now
    if (!view || view.session !== session || !canvas) return;
    frameSize = { width: 0, height: 0 };
    if (!webCodecsSupported()) {
      view.error = "Questo sistema non supporta la decodifica video (WebCodecs).";
      return;
    }
    remote = new RemoteVideo(
      canvas,
      () => invoke("request_keyframe", { session }),
      (fps) => {
        if (view) view.fps = fps;
      },
      () => {
        if (view) view.waiting = false;
      },
      (width, height) => (frameSize = { width, height }),
    );
    const frames = new Channel<ArrayBuffer>();
    const video = remote;
    // Acknowledge processed frames in small batches: the backend stops sending when we lag
    // (e.g. window in the background) instead of queueing minutes of video.
    let consumed = 0;
    let ackTimer: ReturnType<typeof setTimeout> | undefined;
    const flushAck = () => {
      ackTimer = undefined;
      if (consumed > 0) {
        invoke("frames_consumed", { session, count: consumed });
        consumed = 0;
      }
    };
    frames.onmessage = (message) => {
      video.push(message);
      consumed += 1;
      if (consumed >= 5) flushAck();
      else ackTimer ??= setTimeout(flushAck, 50);
    };
    // Events that happened before this window was ready (size, monitors, path...).
    const missed = await invoke<UiEvent[] | null>("attach_view", { session, frames });
    if (missed === null) {
      endVideo("la sessione è già terminata");
      return;
    }
    for (const event of missed) handle(event);
    canvas.focus();
  }

  // ---- Remote control: viewer input → partner ----------------------------------------
  const buttons = ["left", "middle", "right", "back", "forward"];
  const keysDown = new Set<string>();
  /** Mouse buttons pressed on the picture, so their release is sent too. */
  const buttonsDown = new Set<string>();
  let pendingMove: { x: number; y: number } | null = null;
  let moveScheduled = false;
  let wheelAccum = { x: 0, y: 0 };

  function sendInput(event: InputEvent) {
    if (!view || !view.control) return;
    invoke("send_input", { session: view.session, event });
  }

  /** Area showing the picture: the canvas is sized to it (`picture`), centred in the window. */
  function pictureRect() {
    return canvas!.getBoundingClientRect();
  }

  /** Pointer position normalized to the displayed image. */
  function normalized(e: MouseEvent) {
    const r = pictureRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)),
      y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)),
    };
  }

  /** Over the picture, not over the black bars around it. */
  function onPicture(e: MouseEvent) {
    const r = pictureRect();
    return (
      e.clientX >= r.left && e.clientX <= r.left + r.width && e.clientY >= r.top && e.clientY <= r.top + r.height
    );
  }

  /** Modifier keys and the event flag telling whether they are still held. */
  const modifiers: [string, (e: MouseEvent | KeyboardEvent) => boolean][] = [
    ["MetaLeft", (e) => e.metaKey],
    ["MetaRight", (e) => e.metaKey],
    ["ShiftLeft", (e) => e.shiftKey],
    ["ShiftRight", (e) => e.shiftKey],
    ["ControlLeft", (e) => e.ctrlKey],
    ["ControlRight", (e) => e.ctrlKey],
    ["AltLeft", (e) => e.altKey],
    ["AltRight", (e) => e.altKey],
  ];

  /**
   * Releases on the partner the modifiers no longer held here. Shortcuts taken by this
   * system (⌘⇧4 screenshot, ⌘Tab...) swallow the key-up, and ⌘⇧ then stayed down on the
   * partner: every click and key became a shortcut.
   */
  function syncModifiers(e: MouseEvent | KeyboardEvent) {
    for (const [code, held] of modifiers) {
      if (keysDown.has(code) && !held(e)) {
        keysDown.delete(code);
        sendInput({ k: "key", code, down: false });
      }
    }
  }

  function onMouseMove(e: MouseEvent) {
    syncModifiers(e);
    pendingMove = normalized(e);
    if (moveScheduled) return;
    moveScheduled = true;
    requestAnimationFrame(() => {
      moveScheduled = false;
      if (pendingMove) sendInput({ k: "mouse_move", ...pendingMove });
      pendingMove = null;
    });
  }

  function onMouseButton(e: MouseEvent, down: boolean) {
    e.preventDefault();
    canvas?.focus();
    syncModifiers(e);
    const button = buttons[e.button];
    if (!button) return;
    // A click on the black bars must not land on the partner's screen edge; the release
    // of a press made on the picture is always sent, wherever the pointer ends up.
    if (down) {
      if (!onPicture(e)) return;
      buttonsDown.add(button);
    } else if (!buttonsDown.delete(button)) {
      return;
    }
    sendInput({ k: "mouse_move", ...normalized(e) });
    sendInput({ k: "mouse_button", button, down });
  }

  function onWheel(e: WheelEvent) {
    // Without control the wheel scrolls a 1:1 picture larger than the window.
    if (!view?.control) return;
    e.preventDefault();
    // Pixel deltas (trackpads) arrive in small steps: one notch every ~100 px.
    const unit = e.deltaMode === 1 ? 3 : e.deltaMode === 2 ? 1 / 10 : 100;
    wheelAccum.x += e.deltaX / unit;
    wheelAccum.y += e.deltaY / unit;
    const dx = Math.trunc(wheelAccum.x);
    const dy = Math.trunc(wheelAccum.y);
    if (dx === 0 && dy === 0) return;
    wheelAccum.x -= dx;
    wheelAccum.y -= dy;
    sendInput({ k: "wheel", dx, dy });
  }

  function onKey(e: KeyboardEvent, down: boolean) {
    if (!view?.control || !e.code) return;
    e.preventDefault();
    // The key's own flag is not yet/still set on its keydown/keyup: check the others only.
    if (!modifiers.some(([code]) => code === e.code)) syncModifiers(e);
    // Injected keys do not auto-repeat on macOS and Windows: forward this side's repeats
    // (a held arrow keeps moving the cursor). Linux hosts drop them (X repeats by itself).
    if (down) keysDown.add(e.code);
    else keysDown.delete(e.code);
    sendInput({ k: "key", code: e.code, down });
  }

  /** Never leave keys stuck on the partner when focus moves away. */
  function releaseKeys() {
    for (const code of keysDown) sendInput({ k: "key", code, down: false });
    keysDown.clear();
    for (const button of buttonsDown) sendInput({ k: "mouse_button", button, down: false });
    buttonsDown.clear();
  }

  /** This machine can play the partner's sound (not on Linux). */
  let audioSupported = $state(false);

  function toggleBlockInput() {
    if (!view) return;
    // Shown once the partner confirms (it may refuse, e.g. on Linux).
    invoke("set_block_input", { session: view.session, enabled: !view.inputBlocked });
  }

  // Viewer bar (ADR 0067): liquid glass over the partner's screen. After 30 s it shrinks into the
  // status dot (unless pinned); hovering the dot opens a panel with the connection details.
  const readFlag = (key: string) => {
    try {
      return localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  };
  const BAR_AUTO_HIDE_MS = 30_000;
  /** The hint next to the dot is shown the first times only. */
  const BAR_HINT_TIMES = 3;
  let barPinned = $state(readFlag("viewBarPinned"));
  let barHidden = $state(false);
  let barHint = $state(false);
  let panelOpen = $state(false);
  let panelFading = $state(false);
  let barEl = $state<HTMLDivElement | null>(null);
  let blobEl = $state<HTMLDivElement | null>(null);
  let blobBody = $state<HTMLDivElement | null>(null);
  let barHeight = $state(0);
  let panelHeight = $state(220);
  let barHover = false;
  let barTimer: ReturnType<typeof setTimeout> | undefined;
  let panelTimer: ReturnType<typeof setTimeout> | undefined;
  let hintTimer: ReturnType<typeof setTimeout> | undefined;
  let barStartedFor: number | null = null;
  /** The bar stays out while the session is over (Chiudi, reconnecting). */
  const barShown = $derived(!barHidden || !!view?.ended);

  // Springs as CSS linear(): lightly underdamped, so shapes settle like a liquid.
  (function installSprings() {
    const spring = (k: number, c: number) => {
      const dt = 1 / 240;
      let x = 0;
      let v = 0;
      let t = 0;
      const points: number[] = [];
      while (t < 2) {
        v += (-k * (x - 1) - c * v) * dt;
        x += v * dt;
        t += dt;
        points.push(x);
        if (t > 0.3 && Math.abs(x - 1) < 0.0005 && Math.abs(v) < 0.005) break;
      }
      const n = 90;
      const values: string[] = [];
      for (let i = 0; i <= n; i++) {
        values.push(i === 0 ? "0" : i === n ? "1" : points[Math.round((i / n) * (points.length - 1))].toFixed(4));
      }
      return { easing: `linear(${values.join(", ")})`, duration: `${t.toFixed(2)}s` };
    };
    const quick = spring(220, 26);
    const soft = spring(150, 21);
    const supported = CSS.supports("transition-timing-function", quick.easing);
    const root = document.documentElement.style;
    root.setProperty("--spring", supported ? quick.easing : "cubic-bezier(.3, 1.3, .5, 1)");
    root.setProperty("--spring-soft", supported ? soft.easing : "cubic-bezier(.3, 1.2, .45, 1)");
    root.setProperty("--dur", quick.duration);
    root.setProperty("--dur-slow", soft.duration);
  })();

  function scheduleBarHide(ms = BAR_AUTO_HIDE_MS) {
    clearTimeout(barTimer);
    if (barPinned) return;
    barTimer = setTimeout(() => {
      // Not while the pointer is on the bar, or a menu of it is open: try again shortly.
      if (barHover || (document.activeElement && barEl?.contains(document.activeElement))) {
        scheduleBarHide(3000);
        return;
      }
      if (!barPinned && view && !view.ended) hideBar(true);
    }, ms);
  }

  // "Dopo 30 secondi di connessione": counted from the first picture of each session.
  $effect(() => {
    if (view && view.width !== null && barStartedFor !== view.session) {
      barStartedFor = view.session;
      scheduleBarHide();
    }
  });

  function hideBar(auto: boolean) {
    clearTimeout(barTimer);
    barHidden = true;
    panelOpen = false;
    if (!auto) return;
    let shown = 0;
    try {
      shown = Number(localStorage.getItem("viewBarHint") ?? 0);
      localStorage.setItem("viewBarHint", String(shown + 1));
    } catch {}
    if (shown >= BAR_HINT_TIMES) return;
    barHint = true;
    clearTimeout(hintTimer);
    hintTimer = setTimeout(() => (barHint = false), 6000);
  }

  function togglePin() {
    barPinned = !barPinned;
    try {
      if (barPinned) localStorage.setItem("viewBarPinned", "1");
      else localStorage.removeItem("viewBarPinned");
    } catch {}
    if (barPinned) clearTimeout(barTimer);
    else scheduleBarHide();
  }

  function openPanel() {
    clearTimeout(panelTimer);
    if (!barHidden || panelOpen) return;
    barHint = false;
    panelOpen = true;
  }
  function closePanel() {
    clearTimeout(panelTimer);
    panelTimer = setTimeout(() => (panelOpen = false), 60);
  }

  /** "Ripristina barra": the open panel turns into the bar. The bar starts at the panel's size in
   * the same corner and stretches to full width while the panel fades; nothing goes back through
   * the dot. */
  async function restoreBar() {
    clearTimeout(panelTimer);
    barHint = false;
    const bar = barEl;
    const blob = blobEl;
    if (bar && blob && bar.offsetWidth && bar.offsetHeight) {
      const sx = blob.offsetWidth / bar.offsetWidth;
      const sy = blob.offsetHeight / bar.offsetHeight;
      bar.style.transition = "none";
      bar.style.transform = `scale(${sx}, ${sy})`;
      bar.style.opacity = "0";
      bar.getBoundingClientRect();
      panelFading = true;
      barHidden = false;
      await tick();
      bar.style.transition = bar.style.transform = bar.style.opacity = "";
      setTimeout(() => {
        panelOpen = false;
        panelFading = false;
      }, 450);
    } else {
      panelOpen = false;
      barHidden = false;
    }
    scheduleBarHide();
  }

  $effect(() => {
    const body = blobBody;
    if (!body) return;
    const observer = new ResizeObserver(() => (panelHeight = body.scrollHeight + 2));
    observer.observe(body);
    return () => observer.disconnect();
  });

  /** One line on how the connection is doing, for the dot's panel. */
  function verdict(v: View): { level: "ok" | Level; text: string } {
    const { level: l, reasons } = connectionLevel(v);
    if (l === "ok") return { level: l, text: "Connessione ottima" };
    const first = reasons[0].replace(/^(Critico|Attenzione): /, "");
    return { level: l, text: (l === "critical" ? "Connessione in difficoltà: " : "Connessione rallentata: ") + first.toLowerCase() };
  }

  // Chat drawer on the right edge. Pinned, it stays open and the partner's screen fits the space left of it.
  let chatPinned = $state(readFlag("chatPinned"));
  function toggleChatPin() {
    chatPinned = !chatPinned;
    try {
      if (chatPinned) localStorage.setItem("chatPinned", "1");
      else localStorage.removeItem("chatPinned");
    } catch {}
    if (chatPinned && view) openViewChat(view, false);
  }
  let chatLog = $state<HTMLDivElement | null>(null);
  let chatInput = $state<HTMLTextAreaElement | null>(null);
  let peekTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    // Follow new messages.
    const count = view?.chat.length ?? 0;
    const log = chatLog;
    if (count && log) tick().then(() => log.scrollTo({ top: log.scrollHeight, behavior: "smooth" }));
  });
  function openViewChat(v: View, focus = true) {
    v.showChat = true;
    v.chatUsed = true;
    v.chatDismissed = false;
    v.unread = 0;
    v.chatPeek = null;
    panelOpen = false;
    if (focus) tick().then(() => chatInput?.focus());
  }
  function hideViewChat(v: View) {
    if (chatPinned) return;
    v.showChat = false;
    v.chatDismissed = true;
    chatDraft = "";
  }
  function receiveViewChat(v: View, text: string) {
    v.chat.push({ mine: false, text, time: nowTime() });
    if (v.showChat) return;
    // The first message opens the drawer, without taking the keyboard from the remote screen.
    if (!v.chatDismissed && !v.chatUsed) {
      openViewChat(v, false);
      return;
    }
    v.chatUsed = true;
    v.unread += 1;
    v.chatPeek = text.length > 90 ? text.slice(0, 90) + "…" : text;
    clearTimeout(peekTimer);
    peekTimer = setTimeout(() => {
      if (view) view.chatPeek = null;
    }, 4500);
  }
  /** Height of the chat tab on the right edge, 0 (top) – 1 (bottom); dragged by the user, remembered. */
  let chatTabY = $state(
    (() => {
      try {
        const value = Number(localStorage.getItem("chatTabY"));
        return localStorage.getItem("chatTabY") !== null && Number.isFinite(value) ? value : 0.5;
      } catch {
        return 0.5;
      }
    })(),
  );
  const TAB_MIN = 0.08;
  const TAB_MAX = 0.92;
  let tabDrag: { startY: number; start: number; height: number; moved: boolean } | null = null;
  let tabDragged = false;
  function saveTabY() {
    try {
      localStorage.setItem("chatTabY", chatTabY.toFixed(3));
    } catch {}
  }
  function tabDown(e: PointerEvent) {
    const tab = e.currentTarget as HTMLElement;
    tabDrag = { startY: e.clientY, start: chatTabY, height: tab.parentElement?.clientHeight || 1, moved: false };
    tab.setPointerCapture(e.pointerId);
  }
  function tabMove(e: PointerEvent) {
    if (!tabDrag) return;
    const dy = e.clientY - tabDrag.startY;
    // A few pixels of slack: a click with a shaky hand still opens the chat.
    if (Math.abs(dy) > 4) tabDrag.moved = true;
    if (tabDrag.moved) chatTabY = Math.min(TAB_MAX, Math.max(TAB_MIN, tabDrag.start + dy / tabDrag.height));
  }
  function tabUp() {
    if (!tabDrag) return;
    tabDragged = tabDrag.moved;
    tabDrag = null;
    if (tabDragged) saveTabY();
  }
  function tabClick() {
    // The click that ends a drag does not open the chat.
    if (tabDragged) {
      tabDragged = false;
      return;
    }
    if (view) openViewChat(view);
  }
  function tabKey(e: KeyboardEvent) {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    chatTabY = Math.min(TAB_MAX, Math.max(TAB_MIN, chatTabY + (e.key === "ArrowUp" ? -0.05 : 0.05)));
    saveTabY();
  }

  function chatKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      (e.currentTarget as HTMLTextAreaElement).form?.requestSubmit();
    }
  }

  /** Volume of the partner's sound, 0–100, remembered on this computer. */
  let audioVolume = $state(
    (() => {
      try {
        const saved = localStorage.getItem("audioVolume");
        const value = Number(saved);
        return saved !== null && Number.isFinite(value) ? value : 100;
      } catch {
        return 100;
      }
    })(),
  );

  function setVolume(value: number) {
    audioVolume = value;
    try {
      localStorage.setItem("audioVolume", String(value));
    } catch {}
    if (view) invoke("set_audio_volume", { session: view.session, volume: value / 100 });
  }

  function toggleAudio() {
    if (!view) return;
    view.audio = !view.audio;
    if (view.audio) invoke("set_audio_volume", { session: view.session, volume: audioVolume / 100 });
    invoke("set_audio", { session: view.session, enabled: view.audio });
  }

  function toggleHd() {
    if (!view) return;
    view.hd = !view.hd;
    invoke("set_high_resolution", { session: view.session, enabled: view.hd });
  }

  async function selectDisplay(id: number) {
    if (!view || id === view.currentDisplay) return;
    releaseKeys();
    await invoke("select_display", { session: view.session, id });
  }

  function toggleControl() {
    if (!view || view.controlError) return;
    if (view.control) releaseKeys();
    view.control = !view.control;
  }

  /** The session is over: stop the video, keep the window open with the reason. */
  async function endVideo(reason: string) {
    releaseKeys();
    remote?.close();
    remote = null;
    if (view) {
      view.ended = reason;
      view.control = false;
    }
    const win = getCurrentWindow();
    if (await win.isFullscreen()) await win.setFullscreen(false);
  }

  async function openFiles() {
    if (!view) return;
    const peer = view.peerName ? `${view.peerName} (${view.peer})` : view.peer;
    try {
      await invoke("open_files_window", { session: view.session, peer });
    } catch (e) {
      addLog("error", String(e));
    }
  }

  /** Ends the session (if still running) and closes this window. */
  async function disconnect() {
    if (view && !view.ended) await invoke("close_view", { session: view.session });
    if (view?.reconnecting) await invoke("cancel_connect");
    try {
      await getCurrentWindow().close();
    } catch (e) {
      addLog("error", `impossibile chiudere la finestra: ${e}`);
    }
  }

  async function toggleClipboard() {
    if (!view || view.clipboard === null) return;
    await invoke("set_clipboard", { session: view.session, enabled: !view.clipboard });
  }

  async function stopHosting(session: number) {
    await invoke("stop_hosting", { session });
  }

  // Auto-reject when the countdown expires (the backend times out too).
  $effect(() => {
    if (approval && secondsLeft === 0) {
      respond(false);
    }
  });
</script>

{#snippet nameEditor()}
  {#if editingName}
    <form class="value-row" onsubmit={(e) => { e.preventDefault(); saveName(); }}>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="name-input"
        maxlength="40"
        placeholder="es. Alberto – Kronos"
        aria-label="Il tuo nome"
        autofocus
        bind:value={nameDraft}
        onkeydown={(e) => e.key === "Escape" && cancelName()}
      />
      <button class="icon-btn primary" type="submit">Salva</button>
      <button class="icon-btn" type="button" onclick={cancelName}>Annulla</button>
    </form>
  {:else}
    <div class="value-row">
      <span class:muted={!displayName}>{displayName ?? "Nessun nome"}</span>
      <button class="icon-btn" onclick={() => (editingName = true)}>Modifica</button>
    </div>
  {/if}
{/snippet}

{#snippet updatesGroup()}
  <section class="card rows" class:highlight={update.kind === "available"}>
    <div class="pref">
      <div class="pref-text">
        {#if update.kind === "available"}
          <span class="pref-title">È disponibile la versione {update.info.version} <span class="pill">Nuova</span></span>
          <span class="hint small">
            Installata: {appVersion}. L'app si riavvia al termine; su macOS potrebbe servire concedere di nuovo i
            permessi Registrazione schermo e Accessibilità.
          </span>
        {:else if update.kind === "installing"}
          <span class="pref-title">Download della versione {update.info.version}…</span>
          <div class="countdown">
            <div style="width: {update.total ? Math.min(100, (update.downloaded / update.total) * 100) : 5}%"></div>
          </div>
          <span class="hint small">
            {(update.downloaded / 1048576).toFixed(1)} MB{update.total
              ? ` di ${(update.total / 1048576).toFixed(1)} MB`
              : ""}
          </span>
        {:else}
          <span class="pref-title">Versione {appVersion}</span>
          {#if update.kind === "none"}
            <span class="hint small status-line"><span class="dot ok"></span>Hai già l'ultima versione.</span>
          {:else if update.kind === "error"}
            <span class="error">{update.message}</span>
          {:else if update.kind === "checking"}
            <span class="hint small">Controllo in corso…</span>
          {/if}
        {/if}
      </div>
      {#if update.kind === "available"}
        <button class="primary" onclick={installUpdate}>Installa e riavvia</button>
      {:else if update.kind !== "installing"}
        <button class="icon-btn" disabled={update.kind === "checking"} onclick={checkUpdate}>
          Controlla aggiornamenti
        </button>
      {/if}
    </div>
    {#if update.kind === "available" && update.info.notes}
      <div class="notes">
        {#each changesSince(update.info.notes, update.info.current) as change}
          {#if change.title}<strong>Versione {change.title}</strong>{/if}
          <pre>{change.body}</pre>
        {/each}
      </div>
    {/if}
  </section>
{/snippet}

{#if filesSession !== null}
  <FileManager session={filesSession} />
{:else if chatPopupSession !== null}
  <div class="popup-chat">
    <div class="chat-log" bind:this={popupLog}>
      {#each popupChat as m}
        <div class="chat-msg" class:mine={m.mine}>
          <span class="chat-text">{m.text}</span>
          {#if m.time}<span class="chat-time">{m.time}</span>{/if}
        </div>
      {/each}
    </div>
    <form class="chat-input" onsubmit={sendPopupChat}>
      <!-- svelte-ignore a11y_autofocus -->
      <input placeholder="Rispondi a {popupPeer}…" bind:value={popupDraft} maxlength="2000" />
      <button class="primary" type="submit">Invia</button>
    </form>
  </div>
{:else if viewSession !== null && view}
  {@const health = connectionLevel(view)}
  {@const said = verdict(view)}
  {@const resyncCount = recentResyncs(view)}
  <div class="viewer" class:chat-open={view.showChat && !view.ended} class:chat-pinned={chatPinned && view.showChat && !view.ended}>
    {#snippet chatIcon()}
      <svg class="i" viewBox="0 0 24 24"><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z" /></svg>
    {/snippet}
    {#snippet filesIcon()}
      <svg class="i" viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /><path d="m10 13 2-2 2 2M12 11v5" /></svg>
    {/snippet}
    {#snippet powerIcon()}
      <svg class="i" viewBox="0 0 24 24"><path d="M12 3v9" /><path d="M6.3 7.3a8 8 0 1 0 11.4 0" /></svg>
    {/snippet}
    {#if barPinned && barShown}
      <!-- Pinned: the picture starts below the bar instead of under it. -->
      <div class="bar-space" style:height="{barHeight + 16}px"></div>
    {/if}
    <div class="vbar glass" class:gone={!barShown} bind:this={barEl} bind:clientHeight={barHeight}
      onmouseenter={() => (barHover = true)} onmouseleave={() => (barHover = false)}
      role="toolbar" aria-label="Barra della sessione" tabindex="-1">
      <div class="vrow">
        <span class="vdot {health.level}" title={health.reasons.join("\n") || "Connessione in buone condizioni"}></span>
        {#if view.peerName}<span class="vname" title="Nome impostato dal partner">{view.peerName}</span>{/if}
        <span class="vid mono">{view.peer}</span>
        <button type="button" class="vib" class:on={barPinned} aria-pressed={barPinned} onclick={togglePin}
          title={barPinned ? "Barra bloccata in alto. Clic per farla nascondere da sola dopo 30 secondi" : "Blocca la barra in alto (non si nasconde da sola)"}
          aria-label="Blocca la barra">
          <svg class="i" viewBox="0 0 24 24"><path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z" /><path d="M12 14v6" /></svg>
        </button>
        {#if !view.ended}
          <button type="button" class="vib" onclick={() => hideBar(false)}
            title="Nascondi la barra (più spazio per lo schermo); passa sul pallino in alto a sinistra per ripristinarla"
            aria-label="Nascondi la barra">
            <svg class="i" viewBox="0 0 24 24"><path d="m6 15 6-6 6 6" /></svg>
          </button>
        {/if}
        <span class="spacer"></span>
        {#if view.ended && view.reconnecting}
          <span class="badge warn">Ricollegamento…</span>
        {:else if view.ended}
          <span class="badge warn">Sessione terminata</span>
        {:else}
          <div class="vseg">
            {#if view.controlError}
              <span class="badge warn" title={view.controlError}>Solo visione</span>
            {:else}
              <button type="button" class:on={view.control} onclick={toggleControl}
                title={view.control ? "Controllo attivo: mouse e tastiera vanno al partner. Clic per guardare soltanto" : "Solo visione. Clic per inviare mouse e tastiera al partner"}>
                <svg class="i" viewBox="0 0 24 24"><path d="m4 4 7 16 2-7 7-2z" /></svg><span class="lbl">Controllo</span>
              </button>
              {#if view.control}
                <button type="button" class:on={view.inputBlocked} onclick={toggleBlockInput}
                  title={view.inputBlocked
                    ? "Mouse e tastiera del partner bloccati: chi è davanti al suo computer non può usarli (può sbloccarli premendo Esc tre volte). Clic per sbloccarli"
                    : "Blocca mouse e tastiera del partner, così chi è davanti al suo computer non interferisce"}>
                  <svg class="i" viewBox="0 0 24 24"><rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></svg><span class="lbl">Blocca input</span>
                </button>
              {/if}
            {/if}
            <button type="button" class="chat-btn" class:on={view.showChat} class:unread={view.unread > 0}
              onclick={() => view && (view.showChat && !chatPinned ? hideViewChat(view) : openViewChat(view))} title="Scrivi al partner">
              {@render chatIcon()}<span class="lbl">Chat</span>{#if view.unread > 0}<span class="count">{view.unread}</span>{/if}
            </button>
            {#if view.clipboard !== null}
              <button type="button" class:on={view.clipboard} onclick={toggleClipboard}
                title={view.clipboard ? "Appunti condivisi: testo e file copiati su un computer si incollano sull'altro. Clic per spegnerli" : "Appunti spenti. Clic per condividerli"}>
                <svg class="i" viewBox="0 0 24 24"><rect x="7" y="4" width="10" height="4" rx="1" /><path d="M7 6H5v15h14V6h-2" /></svg><span class="lbl">Appunti</span>
              </button>
            {/if}
            <button type="button" onclick={openFiles} title="Sfoglia le cartelle del partner e copia file nei due sensi">
              {@render filesIcon()}<span class="lbl">File</span>
            </button>
            {#if audioSupported}
              <button type="button" class:on={view.audio} onclick={toggleAudio}
                title={view.audio ? "Audio del partner attivo. Clic per spegnerlo" : "Ascolta l'audio del partner (circa 64 kbit/s)"}>
                <svg class="i" viewBox="0 0 24 24"><path d="M11 5 6 9H3v6h3l5 4z" /><path d="M16 9a4 4 0 0 1 0 6" /></svg><span class="lbl">Audio</span>
              </button>
            {/if}
            <button type="button" class:on={view.hd} onclick={toggleHd}
              title={view.hd
                ? "HD attivo: con la rete buona fino a 2560×1600. Clic per limitare a 1080p (meno banda e CPU)"
                : "HD spento: al massimo 1080p. Clic per riattivarlo"}>HD</button>
          </div>
          {#if view.audio && audioSupported}
            <input class="volume" type="range" min="0" max="100" value={audioVolume}
              oninput={(e) => setVolume(Number(e.currentTarget.value))}
              title="Volume: {audioVolume} %" aria-label="Volume dell'audio del partner" />
          {/if}
        {/if}
        {#if view.displays.length > 1}
          <select class="vselect" value={view.currentDisplay}
            onchange={(e) => selectDisplay(Number(e.currentTarget.value))} title="Schermo del partner da visualizzare">
            {#each view.displays as d (d.id)}
              <option value={d.id}>{d.name} · {d.width}×{d.height}{d.primary ? " · principale" : ""}</option>
            {/each}
          </select>
        {/if}
        <select class="vselect" value={shownZoom} onchange={(e) => setZoom(e.currentTarget.value as Zoom)}
          title="Dimensione dell'immagine del partner. Adatta: riempie la finestra, ingrandita con un filtro che mantiene il testo nitido. Ingrandimento intero: ogni pixel del partner diventa un quadrato esatto di pixel del tuo schermo (il più grande che entra). Pixel reali: un pixel del partner per ogni pixel del tuo schermo, con le barre di scorrimento se non entra (senza controllo la rotellina scorre). Risoluzione adattata (solo computer usati da remoto): il partner porta il suo schermo alla risoluzione più vicina alla finestra, poi la ripristina alla fine"
          aria-label="Dimensione dell'immagine">
          <option value="fit">Adatta</option>
          <option value="whole">Intero ×{wholeMultiple}</option>
          <option value="native">Pixel reali</option>
          {#if view.resizable}<option value="remote">Risoluzione adattata</option>{/if}
        </select>
        <button type="button" class="vdisc" onclick={disconnect}>{@render powerIcon()}{view.ended ? "Chiudi" : "Disconnetti"}</button>
      </div>
      <div class="vstats">
        {#if view.path}<span><span class="vpath {view.path}" title={pathHint[view.path]}>{pathLabel[view.path]}</span></span>{/if}
        {#if view.width}
          <span class={level(view.rtt, 150, 300)} title="Latenza: giallo da 150 ms, rosso da 300 ms">Latenza <b>{view.rtt === null ? "–" : `${view.rtt.toFixed(0)} ms`}</b></span>
          <span><b>{view.fps === null ? "–" : view.fps.toFixed(0)} fps</b></span>
          <span title={view.hardware === null ? "" : view.hardware
            ? "GPU: il video è compresso dal chip video del computer controllato"
            : "CPU: il video è compresso dal processore del computer controllato (chip video non disponibile)"}>
            <b>{view.width}×{view.height}{view.hardware === null ? "" : view.hardware ? " GPU" : " CPU"}</b></span>
        {/if}
        {#if view.net}
          <span title="Traffico della sessione: ricevuto ↓ / inviato ↑">↓ <b>{rate(view.net.rx)}</b></span>
          <span>↑ <b>{rate(view.net.tx)}</b></span>
        {/if}
        {#if view.local || view.partner}
          <span class={worst(level(view.local?.cpu, 50, 80), level(view.local?.ram, 800, 1500))}
            title="Risorse usate dall'app su questo computer">Tu <b>{view.local ? `${pct(view.local.cpu)} · ${mb(view.local.ram)}` : "–"}</b></span>
          <span class={worst(level(view.partner?.cpu, 50, 80), level(view.partner?.ram, 800, 1500))}
            title="Risorse usate dall'app sul partner (cattura e codifica avvengono lì)">Partner <b>{view.partner ? `${pct(view.partner.cpu)} · ${mb(view.partner.ram)}` : "–"}</b></span>
        {/if}
        {#if view.limited}
          <span class={view.limited.kbps < 450 ? "critical" : "caution"}
            title="Banda insufficiente: il partner ha ridotto qualità e fotogrammi ({view.limited.fps} fps max) e, sotto 0,8 Mbit/s, la risoluzione. Torna al massimo quando la rete migliora.">
            Qualità ridotta <b>{rate(view.limited.kbps)}</b></span>
        {/if}
        {#if resyncCount > 0}
          <span class={level(resyncCount, 2, 5)}
            title="Volte nell'ultimo minuto in cui l'immagine è stata ricostruita da capo perché si erano persi dati in rete (giallo da 2, rosso da 5)">
            Resync/min <b>{resyncCount}</b></span>
        {/if}
      </div>
      {#if view.transfer}
        <div class="vrow transfer">
          <progress max={view.transfer.total || 1} value={view.transfer.done}></progress>
          <span class="mono small">{transferText(view.transfer)}</span>
          <button type="button" class="vib" onclick={() => view && cancelTransfer(view.session)}
            title="Interrompi il trasferimento" aria-label="Interrompi il trasferimento">✕</button>
        </div>
      {/if}
    </div>

    <div class="screen" bind:this={screenEl}>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <canvas
        bind:this={canvas}
        style:width={picture ? `${picture.cssWidth}px` : null}
        style:height={picture ? `${picture.cssHeight}px` : null}
        tabindex="0"
        class:controlling={view.control}
        onmousemove={onMouseMove}
        onmousedown={(e) => onMouseButton(e, true)}
        onmouseup={(e) => onMouseButton(e, false)}
        oncontextmenu={(e) => e.preventDefault()}
        onwheel={onWheel}
        onkeydown={(e) => onKey(e, true)}
        onkeyup={(e) => onKey(e, false)}
        onblur={releaseKeys}
      ></canvas>
      {#if view.ended && view.reconnecting}
        <div class="reconnect-cover">
          <p>{view.ended.charAt(0).toUpperCase() + view.ended.slice(1)}: ricollegamento in corso…</p>
        </div>
      {:else if view.ended}
        <p class="overlay">Sessione terminata: {view.ended}</p>
      {:else if view.silent}
        <div class="reconnect-cover">
          <p>Il computer remoto non risponde: attendo…</p>
        </div>
      {:else if view.error}
        <p class="overlay error-text">{view.error}</p>
      {:else if view.noScreen}
        <p class="overlay">{view.noScreen}</p>
      {:else if view.waiting}
        <p class="overlay">In attesa dello schermo del partner…</p>
      {/if}
      {#if view.notice}
        <p class="notice">{view.notice}</p>
      {/if}
    </div>

    <!-- The status dot when the bar is hidden; the same shape grows into the panel. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="blob glass" class:shown={!barShown} class:open={panelOpen} class:fading={panelFading}
      bind:this={blobEl} style:--h="{panelHeight}px" tabindex="0" role="group"
      aria-label="Stato della connessione e azioni"
      onmouseenter={openPanel} onmouseleave={closePanel} onfocusin={openPanel}
      onfocusout={(e) => { if (!blobEl?.contains(e.relatedTarget as Node)) closePanel(); }}>
      <span class="vdot {health.level}"></span>
      <div class="blob-body" bind:this={blobBody}>
        <div class="b-head">
          <span class="mono strong">{view.peer}</span>
          {#if view.peerName}<span class="b-name">{view.peerName}</span>{:else}<span class="b-name"></span>{/if}
          {#if view.path}<span class="vpath {view.path}">{pathLabel[view.path]}</span>{/if}
        </div>
        <div class="b-rest">
          <div class="b-verdict {said.level}">{said.text}</div>
          <div class="b-grid">
            <div class="b-cell {level(view.rtt, 150, 300)}"><small>Latenza</small><span>{view.rtt === null ? "–" : `${view.rtt.toFixed(0)} ms`}</span></div>
            <div class="b-cell {view.limited ? 'caution' : ''}"><small>Fotogrammi</small><span>{view.fps === null ? "–" : `${view.fps.toFixed(0)} fps`}</span></div>
            <div class="b-cell"><small>Video</small><span>{view.width ? `${view.width}×${view.height}${view.hardware === null ? "" : view.hardware ? " GPU" : " CPU"}` : "–"}</span></div>
            <div class="b-cell"><small>Ricezione</small><span>{view.net ? rate(view.net.rx) : "–"}</span></div>
            <div class="b-cell {level(resyncCount, 2, 5)}" title="Volte nell'ultimo minuto in cui l'immagine è stata ricostruita perché si erano persi dati in rete"><small>Resync/min</small><span>{resyncCount}</span></div>
            <div class="b-cell {worst(level(view.local?.cpu, 50, 80), level(view.partner?.cpu, 50, 80))}"><small>CPU tu/partner</small><span>{view.local ? pct(view.local.cpu) : "–"} / {view.partner ? pct(view.partner.cpu) : "–"}</span></div>
          </div>
          <div class="b-acts">
            <button type="button" class="b-act" onclick={() => view && openViewChat(view)}>
              {@render chatIcon()}Chat{#if view.unread > 0}<span class="count">{view.unread}</span>{/if}
            </button>
            <button type="button" class="b-act" onclick={openFiles}>{@render filesIcon()}Trasferimento file</button>
            <button type="button" class="b-act restore" onclick={restoreBar}>
              <svg class="i" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></svg>Ripristina barra
            </button>
            <button type="button" class="b-act disc" onclick={disconnect}>{@render powerIcon()}Disconnetti</button>
          </div>
        </div>
      </div>
    </div>
    <div class="bar-hint glass-note" class:show={barHint && !barShown} role="status">
      La barra si è nascosta per lasciarti più spazio. <b>Passa sul pallino</b> per ripristinarla.
    </div>

    {#if !view.ended}
      <aside class="vchat glass" class:open={view.showChat} class:used={view.chatUsed} aria-label="Chat con il partner">
        <button type="button" class="chat-tab" class:unread={view.unread > 0} style:top="{chatTabY * 100}%"
          onclick={tabClick} onpointerdown={tabDown} onpointermove={tabMove} onpointerup={tabUp} onpointercancel={tabUp}
          onkeydown={tabKey}
          aria-label={view.unread > 0 ? `Apri la chat: ${view.unread} messaggi nuovi` : "Apri la chat"}
          title="Clic: apri la chat. Trascina in alto o in basso per spostare la linguetta" tabindex={view.showChat ? -1 : 0}>
          {@render chatIcon()}<span class="tab-badge">{view.unread || ""}</span>
        </button>
        <div class="vchat-head">
          <span class="strong">Chat</span>
          <span class="b-name">{view.peerName ?? view.peer}</span>
          <button type="button" class="vib" class:on={chatPinned} aria-pressed={chatPinned} onclick={toggleChatPin}
            title={chatPinned ? "Chat fissata: resta aperta e lo schermo del partner usa lo spazio a sinistra. Clic per sbloccarla" : "Fissa la chat: resta aperta e lo schermo del partner si stringe accanto a lei"}
            aria-label="Fissa la chat">
            <svg class="i" viewBox="0 0 24 24"><path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z" /><path d="M12 14v6" /></svg>
          </button>
          {#if !chatPinned}
            <button type="button" class="vib" onclick={() => view && hideViewChat(view)}
              title="Nascondi la chat (resta una linguetta sul bordo)" aria-label="Nascondi la chat">
              <svg class="i" viewBox="0 0 24 24"><path d="m9 6 6 6-6 6" /></svg>
            </button>
          {/if}
        </div>
        <div class="vchat-log" bind:this={chatLog} aria-live="polite">
          {#each view.chat as m, i (i)}
            <div class="vmsg" class:mine={m.mine}>
              <div class="bubble">{m.text}</div>
              {#if m.time}<time>{m.time}</time>{/if}
            </div>
          {/each}
          {#if view.chat.length === 0}<p class="vchat-empty">Nessun messaggio. Scrivi al partner: lo vede nella sua finestrella di chat.</p>{/if}
        </div>
        <form class="vchat-compose" onsubmit={(e) => view && sendChat(view, e)}>
          <textarea rows="1" placeholder="Scrivi un messaggio" bind:value={chatDraft} bind:this={chatInput}
            maxlength="2000" onkeydown={chatKey} aria-label="Messaggio"></textarea>
          <button type="submit" class="vsend" aria-label="Invia">
            <svg class="i" viewBox="0 0 24 24"><path d="M5 12h14M13 6l6 6-6 6" /></svg>
          </button>
        </form>
      </aside>
      <div class="chat-peek glass-note" class:show={!!view.chatPeek && !view.showChat} style:top="calc(8px + {chatTabY} * (100% - 16px))">
        <b>{view.peerName ?? view.peer}</b>{view.chatPeek ?? ""}
      </div>
    {/if}
  </div>
{:else if viewSession === null && chatPopupSession === null}
<div class="app" class:quick>
  <nav class="sidebar">
    <div class="brand">
      <img src="/icon.svg" alt="" width="30" height="30" />
      <span class="brand-name">{appName}</span>
    </div>
    <div class="nav-items">
      {#each navItems as item (item.id)}
        <button class="nav-item" class:selected={section === item.id} onclick={() => (section = item.id)}
          title={item.label}>
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html icons[item.id]}</svg>
          <span class="nav-label">{item.label}</span>
          {#if item.id === "chat" && hostUnread > 0}<span class="chat-badge">{hostUnread}</span>{/if}
        </button>
      {/each}
    </div>
    <div class="nav-bottom">
      {#if mode === "full"}
        <button class="nav-item" class:selected={section === "preferences"} onclick={() => openPreferences()}
          title="Preferenze e aggiornamenti">
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html icons.preferences}</svg>
          <span class="nav-label">Preferenze</span>
          {#if update.kind === "available"}<span class="update-dot" title="Nuova versione {update.info.version}"></span>{/if}
        </button>
      {/if}
      <span class="nav-status {status.link}" title={status.error ?? linkLabel[status.link]}>
        <span class="dot"></span><span class="nav-label">{linkLabel[status.link]}</span>
      </span>
      {#if appVersion}<span class="nav-version nav-label">v{appVersion}</span>{/if}
    </div>
  </nav>

  <div class="content">
  {#each hosting as h (h.session)}
    <div class="sharing">
      <span class="live"></span>
      Il tuo schermo è condiviso con <strong class="mono">{h.peer}</strong>
      {#if h.path}
        <span class="badge path {h.path}" title={pathHint[h.path]}>{pathLabel[h.path]}</span>
      {/if}
      {#if h.transfer}
        <progress class="banner-progress" max={h.transfer.total || 1} value={h.transfer.done}></progress>
        <span class="mono small">{transferText(h.transfer)}</span>
        <button class="icon-btn cancel-transfer" onclick={() => cancelTransfer(h.session)}
          title="Interrompi il trasferimento" aria-label="Interrompi il trasferimento">✕</button>
      {/if}
      {#if h.blocked}
        <span class="badge warn" title="Chi ti controlla ha bloccato mouse e tastiera di questo computer">
          Mouse e tastiera bloccati · premi Esc tre volte per riprenderli
        </span>
      {/if}
      <span class="spacer"></span>
      <button class="icon-btn" onclick={() => openHostChat(h.session)} title="Scrivi a chi ti controlla">
        Chat{#if h.unread > 0} <span class="chat-badge">{h.unread}</span>{/if}
      </button>
      <button class="danger" onclick={() => stopHosting(h.session)}>Termina</button>
    </div>
  {/each}

  {#if mode}
  {#if section === "support"}
  <main class="home">
    {#if !quick}
      <div class="page-head">
        <h1>Supporto remoto</h1>
        <span class="hint small">Fatti aiutare o aiuta qualcuno</span>
      </div>
    {/if}
    <div class="home-grid">
    <section class="card home-card">
      <div class="card-head">
        <span class="card-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24"><path d="m8 17-5-5 5-5"/><path d="M3 12h13a5 5 0 0 1 0 10h-2"/></svg>
        </span>
        <div>
          <h2>Consenti il controllo</h2>
          <span class="hint small">Comunica ID e password a chi ti assiste</span>
        </div>
      </div>

      <div class="credentials">
        <div class="cred-row">
          <div class="cred">
            <span class="label">Il tuo ID</span>
            <span class="cred-id mono">{status.id ?? "— — —"}</span>
          </div>
          <button class="square-btn" onclick={() => copy(status.id, "id")} disabled={!status.id}
            aria-label="Copia ID" title={copied === "id" ? "Copiato" : "Copia ID"}>
            {#if copied === "id"}<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12 5 5 9-10"/></svg>{:else}<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>{/if}
          </button>
        </div>
        <div class="cred-row">
          <div class="cred">
            <span class="label">Password</span>
            <span class="cred-pw mono">{status.password}</span>
          </div>
          <button class="square-btn" onclick={() => copy(status.password, "pw")}
            aria-label="Copia password" title={copied === "pw" ? "Copiata" : "Copia password"}>
            {#if copied === "pw"}<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12 5 5 9-10"/></svg>{:else}<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>{/if}
          </button>
          <button class="square-btn" onclick={newPassword} aria-label="Nuova password" title="Genera una nuova password">
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M21 12a9 9 0 1 1-2.64-6.36L21 8"/><path d="M21 3v5h-5"/></svg>
          </button>
        </div>
      </div>
      <p class="hint small">Con questa password ogni connessione va confermata da te.</p>

      {#if quick}
        <span class="label">Il tuo nome</span>
        {@render nameEditor()}
        <p class="hint small">Lo vede chi ti controlla e chi controlli, accanto al tuo ID.</p>
      {/if}

      {#if permissions?.required && (!permissions.screen || !permissions.accessibility)}
        <div class="permissions">
          <strong>Permessi macOS mancanti</strong>
          {#if !permissions.screen}
            <div class="perm-row">
              <span>Registrazione schermo: serve per farti vedere</span>
              <button class="icon-btn" onclick={() => invoke("open_permission_settings", { kind: "screen" })}>
                Apri impostazioni
              </button>
            </div>
          {/if}
          {#if !permissions.accessibility}
            <div class="perm-row">
              <span>Accessibilità: serve per farti controllare</span>
              <button class="icon-btn"
                onclick={() => invoke("open_permission_settings", { kind: "accessibility" })}>
                Apri impostazioni
              </button>
            </div>
          {/if}
          <span class="hint small">
            Attiva "{appName}" nell'elenco. Per la Registrazione schermo macOS può chiedere
            di riaprire l'app.
          </span>
        </div>
      {/if}

      {#if uac?.supported}
        <span class="label">Controllo finestre di amministratore (UAC)</span>
        {#if uac.toggleable}
          <p class="hint small">
            Permette a chi ti assiste di usare anche le finestre di amministratore di Windows e la schermata
            di blocco. Con la password personale il PC resta raggiungibile anche dopo un riavvio, prima che
            qualcuno entri in Windows. Richiede i permessi di amministratore una volta.
          </p>
          <div class="row">
            {#if uac.active && uac.outdated}
              <button class="primary attention" disabled={uacBusy} onclick={updateUac}>Aggiorna</button>
              <button class="icon-btn" disabled={uacBusy} onclick={toggleUac}>Disattiva</button>
              <span class="muted small">attivo, da aggiornare per il collegamento dopo il riavvio</span>
            {:else if uac.active}
              <button class="icon-btn" disabled={uacBusy} onclick={toggleUac}>Disattiva</button>
              <span class="muted small">attivo</span>
            {:else}
              <button class="primary attention" disabled={uacBusy} onclick={toggleUac}>Attiva</button>
              <span class="muted small">non attivo</span>
            {/if}
          </div>
        {:else if uac.active}
          <p class="hint small">Attivo: puoi assistere anche sulle finestre di amministratore e sulla schermata di blocco.</p>
        {:else}
          <p class="hint small">
            Per controllare anche le finestre di amministratore, riavvia come amministratore. Altrimenti continui
            con i permessi utente (le finestre di amministratore non saranno visibili).
          </p>
          <div class="row">
            <button class="primary attention" onclick={relaunchAdmin}>Riavvia come amministratore</button>
          </div>
        {/if}
      {/if}

      {#if quick}
        <p class="hint small">
          Accesso rapido: nessuna installazione e l'ID resta sempre lo stesso. Chiudi questa finestra per
          terminare la condivisione.
        </p>
      {/if}
      <div class="unattended-row">
        <span class="shield" class:on={status.unattended} aria-hidden="true">
          <svg viewBox="0 0 24 24"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>{#if status.unattended}<path d="m9 12 2 2 4-4"/>{/if}</svg>
        </span>
        <div class="pref-text">
          <span class="unattended-title">
            <span class="pref-title">Accesso non presidiato</span>
            <span class="state-pill" class:on={status.unattended}>{status.unattended ? "Attivo" : "Spento"}</span>
          </span>
          <span class="hint small">
            {#if status.unattended}
              Con la password personale si entra senza conferma
            {:else}
              Con una password personale entri senza conferma{#if quick}, finché questa finestra è aperta{/if}
            {/if}
          </span>
        </div>
        {#if status.unattended}
          <div class="menu-anchor">
            <button class="icon-btn more" type="button" aria-label="Opzioni dell'accesso non presidiato"
              aria-haspopup="menu" aria-expanded={unattendedMenu} onclick={() => (unattendedMenu = !unattendedMenu)}>
              <svg viewBox="0 0 24 24"><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></svg>
            </button>
            {#if unattendedMenu}
              <div class="popup-menu" role="menu">
                <button type="button" role="menuitem" onclick={() => {
                  unattendedMenu = false;
                  personalDialog = { first: "", second: "", error: "" };
                }}>
                  <svg viewBox="0 0 24 24"><circle cx="8" cy="15" r="4" /><path d="m10.8 12.2 8.2-8.2M17 6l2 2M15 8l2 2" /></svg>
                  Cambia password personale
                </button>
                <button type="button" role="menuitem" class="danger-item" onclick={() => {
                  unattendedMenu = false;
                  disablePersonal();
                }}>
                  <svg viewBox="0 0 24 24"><path d="M12 3v9" /><path d="M6.3 7.3a8 8 0 1 0 11.4 0" /></svg>
                  Disattiva
                </button>
              </div>
            {/if}
          </div>
        {:else}
          <button class="icon-btn primary-small" disabled={!status.id}
            onclick={() => (personalDialog = { first: "", second: "", error: "" })}>Attiva</button>
        {/if}
      </div>
      {#if quick}
        <span class="label">Chi si collega a questo computer può</span>
        <label class="check">
          <input type="checkbox" checked={hostPermissions.clipboard}
            onchange={() => setHostPermission("clipboard", !hostPermissions.clipboard)} />
          Condividere gli appunti
        </label>
        <label class="check">
          <input type="checkbox" checked={hostPermissions.files}
            onchange={() => setHostPermission("files", !hostPermissions.files)} />
          Usare il Trasferimento file
        </label>
      {:else}
        <p class="hint small prefs-link">
          {#if status.unattended && !autostart}
            Conviene attivare <strong>Avvia all'accesso</strong>: dopo un riavvio il computer resta raggiungibile.
          {:else}
            Nome, permessi di chi si collega e computer senza monitor:
          {/if}
          <button class="link" type="button" onclick={() => openPreferences()}>Preferenze</button>
        </p>
      {/if}
    </section>

    {#if !quick}
    <section class="card home-card">
      <div class="card-head">
        <span class="card-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/></svg>
        </span>
        <div>
          <h2>Controlla un computer remoto</h2>
          <span class="hint small">Inserisci ID e password del partner</span>
        </div>
      </div>
      <form class="connect-form" onsubmit={connect}>
        <label>
          <span class="label">ID partner</span>
          <input
            class="mono partner-id"
            inputmode="numeric"
            placeholder="123 456 789"
            value={partnerId}
            oninput={(e) => (partnerId = formatId(e.currentTarget.value))}
          />
        </label>
        <label>
          <span class="label">Password</span>
          <input
            class="mono"
            type="password"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            placeholder={usePersonal ? "password personale del partner" : "password del partner"}
            value={partnerPassword}
            oninput={(e) =>
              (partnerPassword = usePersonal ? e.currentTarget.value : e.currentTarget.value.toLowerCase())}
          />
        </label>
        <div class="segmented wide" role="group" aria-label="Tipo di password">
          <button type="button" aria-pressed={!usePersonal} class:on={!usePersonal}
            title="Il partner conferma la connessione" onclick={() => (usePersonal = false)}>Con conferma</button>
          <button type="button" aria-pressed={usePersonal} class:on={usePersonal}
            title="Password personale del partner: si entra senza conferma" onclick={() => (usePersonal = true)}>
            Password personale
          </button>
        </div>
        {#if usePersonal}
          <label>
            <span class="label">Salva in "I miei dispositivi" come (facoltativo)</span>
            <input placeholder="es. PC ufficio" bind:value={saveAs} />
          </label>
        {/if}
        {#if formError}<p class="error">{formError}</p>{/if}
        {#if busy}
          <div class="row connecting">
            <span class="spinner"></span>
            <span class="spacer">Connessione in corso…</span>
            <button type="button" class="icon-btn" onclick={cancelConnect}>Annulla</button>
          </div>
        {:else}
          <button
            class="primary connect-btn"
            type="submit"
            disabled={status.link !== "online" || partnerId.length < 11 || !partnerPassword}
          >
            Connetti
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14"/><path d="m13 6 6 6-6 6"/></svg>
          </button>
        {/if}
      </form>
    </section>
    {/if}
    </div>

    {#if !quick && devices.length > 0}
      <section class="card quick-devices" aria-labelledby="quick-devices-title">
        <div class="quick-devices-head">
          <h2 id="quick-devices-title" class="group-title">I miei dispositivi</h2>
          <button class="link" type="button" onclick={() => (section = "devices")}>Tutti →</button>
        </div>
        <div class="quick-devices-list">
          {#each homeDevices as d (d.id)}
            {@const online = deviceOnline(d)}
            <button type="button" class="quick-device" disabled={busy || status.link !== "online"}
              title="Connetti a {d.name}" onclick={() => connectDevice(d)}>
              <span class="dot" class:ok={online === true} class:off={online !== true}></span>
              <span class="quick-device-text">
                <span class="quick-device-name">{d.name}</span>
                <span class="hint small mono">{d.id}</span>
              </span>
              <span class="small" class:ok-text={online === true} class:muted={online !== true}>
                {online === true ? "Online" : online === false ? "Offline" : ""}
              </span>
            </button>
          {/each}
        </div>
      </section>
    {/if}
  </main>
  {:else if section === "devices"}
  {#snippet tree(parent: string | null, depth: number)}
    {#each childGroups(parent) as g (g.id)}
      {@const count = folderCount(g.id)}
      {@const sel = organizing ? folderPicked(g) : false}
      <li class="folder" style="--depth: {depth}">
        {#if organizing}
          <input type="checkbox" class="pick" title="Seleziona tutti i dispositivi della cartella"
            checked={sel === true} indeterminate={sel === "some"} disabled={count.total === 0}
            onchange={() => togglePickedFolder(g)} />
        {/if}
        <button class="folder-toggle" aria-expanded={!g.collapsed} onclick={() => toggleGroup(g)}>
          <svg class="chevron" class:open={!g.collapsed} viewBox="0 0 24 24" width="14" height="14" fill="none"
            stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"
            aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linejoin="round" aria-hidden="true">
            <path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
          <span class="strong">{g.name}</span>
          <span class="muted small">
            {count.total} {count.total === 1 ? "dispositivo" : "dispositivi"}{#if count.online > 0}{` · ${count.online} online`}{/if}
          </span>
        </button>
        <span class="spacer"></span>
        {#if organizing}
        <button class="icon-btn" title="Nuova cartella dentro {g.name}"
          onclick={() => (groupDialog = { id: null, name: "", parent: g.id, error: "" })}>+ Cartella</button>
        <button class="icon-btn" title="Rinomina o sposta la cartella"
          onclick={() => (groupDialog = { id: g.id, name: g.name, parent: g.parent ?? null, error: "" })}>
          Modifica
        </button>
        <button class="icon-btn" onclick={() => deleteGroup(g)}>Elimina</button>
        {/if}
      </li>
      {#if !g.collapsed}
        {#if childGroups(g.id).length === 0 && devicesIn(g.id).length === 0}
          <li class="empty muted small" style="--depth: {depth + 1}">Cartella vuota</li>
        {/if}
        {@render tree(g.id, depth + 1)}
      {/if}
    {/each}
    {#each devicesIn(parent) as d (d.id)}
      {@const online = deviceOnline(d)}
      <li style="--depth: {depth}" class:picked={organizing && picked.includes(d.id)}>
        {#if organizing}
          <input type="checkbox" class="pick" aria-label="Seleziona {d.name}"
            checked={picked.includes(d.id)} onchange={() => togglePicked(d)} />
        {/if}
        {#if online !== null}
          <span class="presence" class:on={online}
            title={online ? "Acceso e raggiungibile" : "Spento o senza connessione"}
          >{online ? "Online" : "Offline"}</span>
        {/if}
        <span class="strong">{d.name}</span>
        <span class="mono muted small">{d.id}</span>
        <span class="spacer"></span>
        {#if !organizing}
        <button class="icon-btn" disabled={busy || status.link !== "online"}
          onclick={() => connectDevice(d)}>Connetti</button>
        <button class="icon-btn" onclick={() => removeDevice(d)}>Rimuovi</button>
        {/if}
      </li>
    {/each}
  {/snippet}
  <main>
    <h1>Dispositivi</h1>
    {#if busy}
      <div class="row connecting">
        <span class="spinner"></span>
        <span class="spacer">Connessione in corso…</span>
        <button type="button" class="icon-btn" onclick={cancelConnect}>Annulla</button>
      </div>
    {/if}
    <section class="card">
      <div class="card-title">
        <h2>I miei dispositivi</h2>
        {#if organizing}
          <div class="row-actions">
            <button class="icon-btn" onclick={() => (groupDialog = { id: null, name: "", parent: null, error: "" })}>
              Nuova cartella
            </button>
            <button class="icon-btn" disabled={picked.length === 0} onclick={openMoveDialog}>
              Sposta in…{picked.length > 0 ? ` (${picked.length})` : ""}
            </button>
            <button class="icon-btn" title="Salva la lista in un file" onclick={openExport}>Esporta</button>
            <button class="icon-btn" title="Aggiungi i dispositivi di un file esportato"
              onclick={() => importInput?.click()}>Importa</button>
            <button class="icon-btn primary" onclick={stopOrganizing}>Fine</button>
          </div>
        {:else if devices.length > 0 || groups.length > 0}
          <button class="icon-btn" title="Crea cartelle, sposta, esporta e importa i dispositivi"
            onclick={startOrganizing}>
            Organizza
          </button>
        {:else}
          <button class="icon-btn" title="Aggiungi i dispositivi di un file esportato"
            onclick={() => importInput?.click()}>Importa</button>
        {/if}
        <input class="hidden-file" type="file" accept=".json,application/json" bind:this={importInput}
          onchange={pickImportFile} />
      </div>
      {#if organizing}
        <p class="hint small">
          Seleziona i dispositivi (o una cartella intera) e premi "Sposta in…". Con "Esporta" salvi la lista in
          un file, con "Importa" aggiungi quella di un altro computer. Premi "Fine" quando hai finito.
        </p>
      {/if}
      {#if devicesMessage}
        <p class="small" class:error={devicesMessage.error} class:hint={!devicesMessage.error}>{devicesMessage.text}</p>
      {/if}
      {#if lastMove}
        <div class="undo-bar small">
          <span class="spacer">{lastMove.message}</span>
          <button class="icon-btn" onclick={undoMove}>Annulla</button>
        </div>
      {/if}
      {#if devices.length === 0 && groups.length === 0}
        <p class="hint small">
          Nessun dispositivo salvato. Collegandoti con la password personale del partner puoi salvarlo qui
          (Supporto remoto → "Salva in I miei dispositivi").
        </p>
      {:else}
        <ul class="devices tree">
          {@render tree(null, 0)}
        </ul>
      {/if}
    </section>
    <section class="card">
      <h2>In rete locale</h2>
      <p class="hint small">Sulla stessa rete: collegamento diretto, senza passare dal server.</p>
      {#if lanPeers.length === 0}
        <p class="hint small">Nessun dispositivo trovato in questa rete.</p>
      {:else}
        <ul class="devices">
          {#each lanPeers as p (p.id)}
            <li>
              <span class="lan-name">
                <span class="strong">{p.alias ?? p.host}</span>
                <span class="muted small">
                  {#if p.alias}{p.host} · {/if}<span class="mono">{formatId(p.id)}</span>
                </span>
              </span>
              <span class="spacer"></span>
              <button class="icon-btn" title="Inserisci l'ID in Supporto remoto"
                onclick={() => {
                  partnerId = formatId(p.id);
                  section = "support";
                }}>Seleziona</button>
              <button class="icon-btn" title="Dai un nome a questo dispositivo (visibile solo su questo computer)"
                onclick={() => (renameDialog = { id: p.id, host: p.host, name: p.alias ?? "", error: "" })}>
                Rinomina
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </main>

  {:else if section === "chat"}
  <main class="chat-page">
    <h1>Chat</h1>
    {#if !activeChat}
      <section class="card">
        <p class="hint">
          Nessuna sessione attiva. Qui scrivi a chi sta controllando questo computer; quando sei tu a
          controllare, usa il pulsante Chat nella finestra della sessione.
        </p>
      </section>
    {:else}
      {#if hosting.length > 1}
        <div class="row">
          {#each hosting as h (h.session)}
            <button class="icon-btn" class:active={h.session === activeChat.session}
              onclick={() => openHostChat(h.session)}>
              <span class="mono">{h.peer}</span>{#if h.unread > 0} <span class="chat-badge">{h.unread}</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
      <div class="chat-panel docked">
        <div class="chat-head">
          <span>Chat con <span class="mono">{activeChat.peer}</span></span>
        </div>
        <div class="chat-log">
          {#each activeChat.chat as m}
            <div class="chat-msg" class:mine={m.mine}>
              <span class="chat-text">{m.text}</span>
              <span class="chat-time">{m.time}</span>
            </div>
          {/each}
          {#if activeChat.chat.length === 0}<p class="hint small">Nessun messaggio.</p>{/if}
        </div>
        <form class="chat-input" onsubmit={(e) => activeChat && sendChat(activeChat, e)}>
          <input placeholder="Scrivi a {activeChat.peer}…" bind:value={chatDraft} maxlength="2000" />
          <button class="primary" type="submit">Invia</button>
        </form>
      </div>
    {/if}
  </main>

  {:else if section === "activity"}
  <main>
    <h1>Attività</h1>
    <section class="card log">
      {#if logs.length === 0}
        <p class="hint">Nessuna attività.</p>
      {:else}
        <ul>
          {#each logs as line}
            <li class={line.level}><span class="time mono">{line.time}</span>{line.message}</li>
          {/each}
        </ul>
      {/if}
    </section>
  </main>

  {:else if section === "preferences"}
  <main class="prefs">
    <h1>Preferenze</h1>
    <div class="tabs" role="tablist" aria-label="Sezioni delle preferenze">
      {#each prefsTabs as t (t.id)}
        <button type="button" role="tab" aria-selected={prefsTab === t.id} class:on={prefsTab === t.id}
          onclick={() => (prefsTab = t.id)}>
          {t.label}
          {#if t.id === "updates" && update.kind === "available"}<span class="update-dot" title="Nuova versione"></span>{/if}
        </button>
      {/each}
    </div>

    {#if prefsTab === "general"}
    <section class="card rows">
      <div class="pref">
        <div class="pref-text">
          <span class="pref-title">Il tuo nome</span>
          <span class="hint small">Lo vede chi ti controlla e chi controlli, accanto al tuo ID.</span>
        </div>
        {@render nameEditor()}
      </div>
      <div class="pref">
        <div class="pref-text">
          <span class="pref-title" id="theme-title">Tema</span>
          <span class="hint small">Sistema segue l'aspetto scelto in {isMac ? "macOS" : "Windows"}.</span>
        </div>
        <div class="segmented" role="group" aria-labelledby="theme-title">
          {#each [["system", "Sistema"], ["light", "Chiaro"], ["dark", "Scuro"]] as [value, label]}
            <button type="button" aria-pressed={theme === value} class:on={theme === value}
              onclick={() => setTheme(value as Theme)}>{label}</button>
          {/each}
        </div>
      </div>
      <div class="pref">
        <div class="pref-text">
          <span class="pref-title">Avvia all'accesso</span>
          <span class="hint small">
            {#if status.unattended && !autostart}
              Consigliato con l'accesso non presidiato: dopo un riavvio il computer resta raggiungibile e chi ti
              assiste si ricollega da solo.
            {:else}
              Parte nascosta nella barra: chi ti assiste ti trova anche dopo un riavvio.
            {/if}
          </span>
        </div>
        <button type="button" class="switch" role="switch" aria-checked={autostart} aria-label="Avvia all'accesso"
          onclick={toggleAutostart}><span></span></button>
      </div>
    </section>

    {:else if prefsTab === "connections"}
    <p class="hint small group-note">Chi si collega a questo computer può:</p>
    <section class="card rows">
      <div class="pref">
        <div class="pref-text">
          <span class="pref-title">Condividere gli appunti</span>
          <span class="hint small">Testo e file copiati passano da un computer all'altro (dalle prossime sessioni).</span>
        </div>
        <button type="button" class="switch" role="switch" aria-checked={hostPermissions.clipboard}
          aria-label="Condividere gli appunti"
          onclick={() => setHostPermission("clipboard", !hostPermissions.clipboard)}><span></span></button>
      </div>
      <div class="pref">
        <div class="pref-text">
          <span class="pref-title">Usare il Trasferimento file</span>
          <span class="hint small">Sfogliare e copiare i file di questo computer.</span>
        </div>
        <button type="button" class="switch" role="switch" aria-checked={hostPermissions.files}
          aria-label="Usare il Trasferimento file"
          onclick={() => setHostPermission("files", !hostPermissions.files)}><span></span></button>
      </div>
    </section>

    {:else if prefsTab === "screen"}
      <section class="card rows">
        {#if uac?.supported || isMac}
          <div class="pref">
            <div class="pref-text">
              <span class="pref-title">Computer usato solo da remoto</span>
              <span class="hint small">
                {#if isMac}
                  Per un Mac raggiunto solo da remoto, senza nessuno davanti a un monitor: chi controlla può
                  cambiare la risoluzione e un Mac senza monitor (Mac mini, Mac Studio) condivide uno schermo virtuale.
                {:else}
                  Per server e macchine virtuali raggiunti solo da remoto: la sessione di Desktop remoto chiusa
                  resta condivisibile e chi controlla può portare lo schermo alla risoluzione della sua finestra.
                {/if}
              </span>
              {#if remoteOnly && uac?.supported && !uac.active}
                <span class="error">Serve il controllo finestre di amministratore (UAC) attivo, in Supporto remoto.</span>
              {:else if uac?.supported}
                <span class="hint small">Vale per tutto il computer: cambiarla chiede il permesso di amministratore.</span>
              {/if}
            </div>
            <button type="button" class="switch" role="switch" aria-checked={remoteOnly}
              aria-label="Computer usato solo da remoto" disabled={!status.unattended && !remoteOnly}
              onclick={() => setRemoteOnly(!remoteOnly)}><span></span></button>
          </div>
        {/if}
        {#if isMac && remoteOnly && virtualSize && virtualSize.sizes.length > 0}
          {@const fixed = virtualSize.fixed}
          <div class="pref column">
            <div class="pref-text">
              <span class="pref-title" id="virtual-size-title">Risoluzione dello schermo virtuale</span>
              <span class="hint small">
                {#if fixed}
                  Fissa: chi controlla vede sempre {fixed[0]} × {fixed[1]}, qualunque sia la sua finestra.
                {:else}
                  Automatica: si adatta alla finestra di chi controlla.
                {/if}
                Vale dalla prossima connessione.
              </span>
            </div>
            <div class="size-grid" role="radiogroup" aria-labelledby="virtual-size-title">
              {#each [null, ...virtualSize.sizes] as size (size ? size.join("x") : "auto")}
                {@const on = size ? fixed?.[0] === size[0] && fixed?.[1] === size[1] : !fixed}
                <button type="button" role="radio" aria-checked={on} class="size-tile" class:on
                  onclick={() => setVirtualSize(size ? size.join("x") : "")}>
                  <span class="size-shape" aria-hidden="true">
                    <span style:width="{size ? 22 + (size[0] - 1280) / 50 : 38}px"
                      style:height="{size ? (22 + (size[0] - 1280) / 50) * size[1] / size[0] : 24}px"></span>
                  </span>
                  <span class="size-label">{size ? `${size[0]} × ${size[1]}` : "Automatica"}</span>
                  <span class="hint small">{size ? sizeName(size) : "finestra di chi controlla"}</span>
                </button>
              {/each}
            </div>
          </div>
        {/if}
        {#if virtualMonitor?.supported}
          <div class="pref">
            <div class="pref-text">
              <span class="pref-title">Monitor virtuale</span>
              <span class="hint small">
                Per un PC senza monitor collegato: aggiunge uno schermo virtuale 1920×1080, così chi si collega vede e
                controlla il desktop. Usa Virtual Display Driver (licenza MIT, firmato da SignPath Foundation);
                Windows chiede il permesso di amministratore.
              </span>
              <span class="small">
                {#if virtualMonitor.installed}
                  Installato{virtualMonitor.version ? ` (versione ${virtualMonitor.version})` : ""}{#if virtualMonitor.bundled && virtualMonitor.version !== virtualMonitor.bundled}
                    · disponibile la versione <strong>{virtualMonitor.bundled}</strong>{/if}.
                {:else}
                  Non installato.
                {/if}
              </span>
              {#if virtualMonitor.message}<span class="hint small">{virtualMonitor.message}</span>{/if}
              {#if virtualMonitorError}<span class="error">{virtualMonitorError}</span>{/if}
            </div>
            <div class="pref-actions">
              {#if virtualMonitorBusy}<span class="spinner"></span>{/if}
              {#if !virtualMonitor.installed}
                <button class="icon-btn" disabled={virtualMonitorBusy} onclick={() => setVirtualMonitor(true)}>Installa</button>
              {:else}
                {#if virtualMonitor.bundled && virtualMonitor.version !== virtualMonitor.bundled}
                  <button class="icon-btn" disabled={virtualMonitorBusy} onclick={() => setVirtualMonitor(true)}>Aggiorna</button>
                {/if}
                <button class="icon-btn" disabled={virtualMonitorBusy} onclick={() => setVirtualMonitor(false)}>Rimuovi</button>
              {/if}
            </div>
          </div>
        {/if}
      </section>
      {#if (uac?.supported || isMac) && !status.unattended && !remoteOnly}
        <p class="hint small group-note">Si attiva insieme all'accesso non presidiato, in Supporto remoto.</p>
      {/if}

    {:else if prefsTab === "updates"}
      {@render updatesGroup()}

    {:else}
    <section class="card rows">
      <div class="pref">
        <div class="pref-text">
          <span class="pref-title">Log dell'app</span>
          <span class="hint small">
            Salva nella cartella Download un file zip con i log degli ultimi giorni, da inviare a chi ti assiste.
          </span>
          {#if logsExport.message}
            <span class={logsExport.error ? "error" : "hint small"}>{logsExport.message}</span>
          {/if}
        </div>
        <div class="pref-actions">
          {#if logsExport.busy}<span class="spinner"></span>{/if}
          <button class="icon-btn" disabled={logsExport.busy} onclick={exportLogs}>Scarica i log</button>
        </div>
      </div>
    </section>
    {/if}
  </main>
  {/if}
  {/if}
  </div>
</div>

{/if}

{#if remoteOnlyAsk}
  <div class="backdrop">
    <div class="modal" role="alertdialog" aria-labelledby="remote-only-title">
      <h2 id="remote-only-title">Computer usato solo da remoto?</h2>
      {#if isMac}
        <p class="hint">
          Rispondi <strong>Sì</strong> per un Mac senza monitor, raggiunto solo da remoto: chi lo controlla può
          portare il suo schermo alla risoluzione della propria finestra (ripristinata alla fine), oppure puoi fissarla nelle Preferenze.
        </p>
        <p class="hint">
          Rispondi <strong>No</strong> per un Mac con un monitor: chi è davanti vedrebbe cambiare la risoluzione.
          Si può cambiare in seguito nelle Preferenze.
        </p>
      {:else}
        <p class="hint">
          Rispondi <strong>Sì</strong> per un server o una macchina virtuale senza monitor, raggiunti solo da remoto
          (Desktop remoto/RDP o Kronos). Chiudendo Desktop remoto senza uscire, la sessione di Windows viene
          spostata sullo schermo del computer, così puoi collegarti con Kronos e vederla; chi controlla può
          inoltre portare lo schermo alla risoluzione della propria finestra (ripristinata alla fine).
        </p>
        <p class="hint">
          Rispondi <strong>No</strong> per un PC con un monitor: spostandola, la sessione comparirebbe sbloccata
          davanti a chiunque sia lì. Si può cambiare in seguito nelle Preferenze.
        </p>
      {/if}
      {#if uac?.supported && !uac?.active}
        <p class="hint small">Serve anche il controllo finestre di amministratore (UAC) attivo.</p>
      {/if}
      <div class="actions">
        <button type="button" onclick={() => setRemoteOnly(false)}>No, ha un monitor</button>
        <button class="primary" type="button" onclick={() => setRemoteOnly(true)}>Sì, solo da remoto</button>
      </div>
    </div>
  </div>
{/if}

{#if confirmDialog}
  <div class="backdrop">
    <div class="modal" role="alertdialog">
      <p>{confirmDialog.message}</p>
      <div class="actions">
        <button type="button" onclick={() => answerConfirm(false)}>Annulla</button>
        <button class="danger" type="button" onclick={() => answerConfirm(true)}>{confirmDialog.action}</button>
      </div>
    </div>
  </div>
{/if}

{#if sessionEnded && viewSession === null}
  <div class="backdrop">
    <div class="modal ended" role="alertdialog" aria-labelledby="ended-title">
      <h2 id="ended-title">Sessione con <span class="mono">{sessionEnded.peer}</span> terminata</h2>
      <div class="ended-duration">
        <span class="ended-duration-label">Durata</span>
        <strong>{formatDuration(sessionEnded.duration)}</strong>
      </div>
      <div class="ended-rows">
        <div class="ended-row">
          <span class="muted small">Ruolo</span>
          <span>{sessionEnded.controlled ? "Hai condiviso il tuo schermo" : "Hai controllato il partner"}</span>
        </div>
        <div class="ended-row">
          <span class="muted small">Terminata alle</span>
          <span>{new Date().toLocaleTimeString("it-IT")}</span>
        </div>
      </div>
      <div class="actions">
        <button class="primary" type="button" onclick={acknowledgeEnded}>OK ({endedLeft})</button>
      </div>
    </div>
  </div>
{/if}

{#if groupDialog}
  <div class="backdrop">
    <form class="modal" onsubmit={saveGroup}>
      <h2>{groupDialog.id === null ? "Nuova cartella" : "Modifica cartella"}</h2>
      <!-- svelte-ignore a11y_autofocus -->
      <input placeholder="es. Clienti" maxlength="40" autofocus bind:value={groupDialog.name} />
      <label>
        <span class="label">Dentro</span>
        <select bind:value={groupDialog.parent}>
          <option value={null}>I miei dispositivi (primo livello)</option>
          {#each folderOptions(groupDialog.id) as f (f.id)}<option value={f.id}>{f.label}</option>{/each}
        </select>
      </label>
      {#if groupDialog.error}<p class="error">{groupDialog.error}</p>{/if}
      <div class="actions">
        <button type="button" onclick={() => (groupDialog = null)}>Annulla</button>
        <button class="primary" type="submit" disabled={!groupDialog.name.trim()}>Salva</button>
      </div>
    </form>
  </div>
{/if}

{#if exportDialog}
  <div class="backdrop">
    <form class="modal" onsubmit={exportDevices}>
      <h2>Esporta la lista dei dispositivi</h2>
      <p class="hint small">Il file viene salvato nella cartella Download.</p>
      <label class="check">
        <input type="checkbox" bind:checked={exportDialog.withPasswords} />
        Includi le password salvate
      </label>
      {#if exportDialog.withPasswords}
        <p class="hint small">
          Con le password il file permette di entrare nei dispositivi senza conferma: viene cifrato con una
          password che ti servirà per importarlo. Senza, contiene solo nomi e cartelle.
        </p>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" placeholder="Password del file (almeno {MIN_FILE_PASSWORD} caratteri)"
          autocomplete="new-password" autofocus bind:value={exportDialog.password} />
        <input type="password" placeholder="Ripeti la password" autocomplete="new-password"
          bind:value={exportDialog.confirm} />
      {:else}
        <p class="hint small">
          Il file conterrà solo nomi e cartelle: dopo l'importazione dovrai collegarti di nuovo con la password
          personale di ogni dispositivo.
        </p>
      {/if}
      {#if exportDialog.error}<p class="error">{exportDialog.error}</p>{/if}
      <div class="actions">
        <button type="button" onclick={() => (exportDialog = null)}>Annulla</button>
        <button class="primary" type="submit" disabled={exportDialog.busy}>
          {exportDialog.busy ? "Esportazione…" : "Esporta"}
        </button>
      </div>
    </form>
  </div>
{/if}

{#if importDialog}
  <div class="backdrop">
    <form class="modal" onsubmit={importLocked}>
      <h2>Importa la lista dei dispositivi</h2>
      <p class="hint small">
        Il file è protetto: scrivi la password scelta quando è stato esportato. I dispositivi già presenti con
        lo stesso ID vengono aggiornati, gli altri aggiunti.
      </p>
      <!-- svelte-ignore a11y_autofocus -->
      <input type="password" placeholder="Password del file" autocomplete="off" autofocus
        bind:value={importDialog.password} />
      {#if importDialog.error}<p class="error">{importDialog.error}</p>{/if}
      <div class="actions">
        <button type="button" onclick={() => (importDialog = null)}>Annulla</button>
        <button class="primary" type="submit" disabled={importDialog.busy || !importDialog.password}>
          {importDialog.busy ? "Importazione…" : "Importa"}
        </button>
      </div>
    </form>
  </div>
{/if}

{#if moveDialog}
  <div class="backdrop">
    <form class="modal" onsubmit={movePicked}>
      <h2>Sposta {picked.length === 1 ? "1 dispositivo" : `${picked.length} dispositivi`} in</h2>
      <div class="folder-choices">
        <label class="choice">
          <input type="radio" name="move-target" value="" bind:group={moveDialog.target} />
          I miei dispositivi (nessuna cartella)
        </label>
        {#each folderOptions() as f (f.id)}
          <label class="choice">
            <input type="radio" name="move-target" value={f.id} bind:group={moveDialog.target} />
            {f.label}
          </label>
        {/each}
      </div>
      {#if groups.length === 0}
        <p class="hint small">Non ci sono ancora cartelle: creane una con "Nuova cartella".</p>
      {/if}
      <div class="actions">
        <button type="button" onclick={() => (moveDialog = null)}>Annulla</button>
        <button class="primary" type="submit">Sposta</button>
      </div>
    </form>
  </div>
{/if}

{#if renameDialog}
  <div class="backdrop">
    <form class="modal" onsubmit={saveLanName}>
      <h2>Nome del dispositivo</h2>
      <p class="hint">
        Computer <strong>{renameDialog.host}</strong> · <span class="mono">{formatId(renameDialog.id)}</span>.
        Il nome è visibile solo su questo computer; lascia vuoto per tornare al nome del computer.
      </p>
      <!-- svelte-ignore a11y_autofocus -->
      <input placeholder="es. PC reception" maxlength="40" autofocus bind:value={renameDialog.name} />
      {#if renameDialog.error}<p class="error">{renameDialog.error}</p>{/if}
      <div class="actions">
        <button type="button" onclick={() => (renameDialog = null)}>Annulla</button>
        <button class="primary" type="submit">Salva</button>
      </div>
    </form>
  </div>
{/if}

{#if personalDialog}
  <div class="backdrop">
    <form class="modal" onsubmit={savePersonalPassword}>
      <h2>Password personale</h2>
      <p class="hint">
        Chi la conosce potrà vedere e controllare questo computer <strong>senza conferma</strong>.
        Almeno 8 caratteri; maiuscole e minuscole sono distinte. Non viene salvata in chiaro.
      </p>
      <input type="password" placeholder="password personale" autocomplete="new-password"
        bind:value={personalDialog.first} />
      <input type="password" placeholder="ripeti la password" autocomplete="new-password"
        bind:value={personalDialog.second} />
      {#if personalDialog.error}<p class="error">{personalDialog.error}</p>{/if}
      <div class="actions">
        <button type="button" onclick={() => (personalDialog = null)}>Annulla</button>
        <button class="primary" type="submit" disabled={!personalDialog.first}>Salva</button>
      </div>
    </form>
  </div>
{/if}

{#if approval}
  <div class="backdrop">
    <div class="modal" role="alertdialog" aria-labelledby="approval-title">
      <h2 id="approval-title">Richiesta di connessione</h2>
      <p>
        Il dispositivo <strong class="mono">{approval.peer}</strong> ha inserito la password corretta
        e vuole <strong>vedere e controllare</strong> questo computer.
      </p>
      <p class="hint">Consenti solo se conosci chi lo sta chiedendo.</p>
      <div class="countdown"><div style="width: {(secondsLeft / approval.total) * 100}%"></div></div>
      <div class="actions">
        <button onclick={() => respond(false)}>Rifiuta ({secondsLeft}s)</button>
        <button class="primary" onclick={() => respond(true)}>Consenti</button>
      </div>
    </div>
  </div>
{/if}

<svelte:window
  onpointerdown={(e) => {
    if (unattendedMenu && !(e.target as Element).closest?.(".menu-anchor")) unattendedMenu = false;
  }}
  onkeydown={(e) => {
    if (unattendedMenu && e.key === "Escape") unattendedMenu = false;
  }}
/>

<style>
  :global(:root) {
    --bg: #f1f5f9;
    --surface: #ffffff;
    --text: #0f172a;
    --muted: #64748b;
    --border: #e2e8f0;
    --accent: #e8830c;
    --accent-2: #f39200;
    /* Dark text on the orange: white on it reads poorly (about 2.6:1). */
    --accent-text: #111827;
    --ok: #16a34a;
    --warn: #d97706;
    --caution: #eab308;
    --err: #dc2626;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif;
    font-size: 15px;
    --switch-off: #cbd5e1;
    --raised: #ffffff;
    /* Liquid glass of the viewer bar, dot panel and chat (ADR 0067). */
    --g-tint: rgba(248, 250, 252, 0.8);
    --g-edge: rgba(255, 255, 255, 0.75);
    --g-dim: rgba(15, 23, 42, 0.68);
    --g-hover: rgba(15, 23, 42, 0.07);
    --g-on: rgba(255, 255, 255, 0.75);
    --g-recess: rgba(15, 23, 42, 0.06);
    --g-sep: rgba(15, 23, 42, 0.12);
    --g-shadow: rgba(15, 23, 42, 0.18);
    --g-note: rgba(255, 255, 255, 0.85);
    --d-line: rgba(220, 38, 38, 0.28);
    --d-bg: rgba(220, 38, 38, 0.1);
    --d-text: #b91c1c;
    --ok-text: #15803d;
    --caution-text: #a16207;
    color: var(--text);
    background: var(--bg);
    color-scheme: light;
  }
  /* Dark: the system's choice unless the preferences force light (data-theme on <html>). */
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) {
      --bg: #0b1120;
      --surface: #111827;
      --text: #e5e7eb;
      --muted: #94a3b8;
      --border: #1f2937;
      --accent: #f39200;
      --accent-2: #ffb347;
      --switch-off: #334155;
      --raised: #1f2937;
      --g-tint: rgba(12, 18, 32, 0.45);
      --g-edge: rgba(255, 255, 255, 0.16);
      --g-dim: rgba(229, 231, 235, 0.74);
      --g-hover: rgba(255, 255, 255, 0.12);
      --g-on: rgba(255, 255, 255, 0.07);
      --g-recess: rgba(0, 0, 0, 0.18);
      --g-sep: rgba(255, 255, 255, 0.12);
      --g-shadow: rgba(0, 0, 0, 0.28);
      --g-note: rgba(13, 19, 33, 0.8);
      --d-line: rgba(248, 113, 113, 0.3);
      --d-bg: rgba(220, 38, 38, 0.22);
      --d-text: #fecaca;
      --ok-text: #4ade80;
      --caution-text: #facc15;
      color-scheme: dark;
    }
  }
  :global(:root[data-theme="dark"]) {
    --bg: #0b1120;
    --surface: #111827;
    --text: #e5e7eb;
    --muted: #94a3b8;
    --border: #1f2937;
    --accent: #f39200;
    --accent-2: #ffb347;
    --switch-off: #334155;
    --raised: #1f2937;
    --g-tint: rgba(12, 18, 32, 0.45);
    --g-edge: rgba(255, 255, 255, 0.16);
    --g-dim: rgba(229, 231, 235, 0.74);
    --g-hover: rgba(255, 255, 255, 0.12);
    --g-on: rgba(255, 255, 255, 0.07);
    --g-recess: rgba(0, 0, 0, 0.18);
    --g-sep: rgba(255, 255, 255, 0.12);
    --g-shadow: rgba(0, 0, 0, 0.28);
    --g-note: rgba(13, 19, 33, 0.8);
    --d-line: rgba(248, 113, 113, 0.3);
    --d-bg: rgba(220, 38, 38, 0.22);
    --d-text: #fecaca;
    --ok-text: #4ade80;
    --caution-text: #facc15;
    color-scheme: dark;
  }
  :global(body) {
    margin: 0;
    background: var(--bg);
  }
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }
  /* Sidebar: same dark tone in light and dark theme, Kronos orange for the selected page. */
  .sidebar {
    --side-bg: #151b2e;
    --side-text: #cbd5e1;
    --side-hover: rgba(255, 255, 255, 0.07);
    width: 220px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 10px;
    box-sizing: border-box;
    background: var(--side-bg);
    color: var(--side-text);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px 14px;
    margin-bottom: 6px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.12);
  }
  .brand-name {
    font-size: 15px;
    font-weight: 700;
    color: #fff;
    line-height: 1.2;
  }
  .nav-items,
  .nav-bottom {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .nav-bottom {
    margin-top: auto;
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 12px;
    border: none;
    border-radius: 9px;
    background: transparent;
    color: var(--side-text);
    font-size: 14px;
    text-align: left;
  }
  .nav-item:hover {
    background: var(--side-hover);
    color: #fff;
  }
  .nav-item.selected {
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }
  .nav-item svg {
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .nav-label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .update-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--accent-2);
    box-shadow: 0 0 0 3px rgba(243, 146, 0, 0.25);
  }
  .nav-item.selected .update-dot {
    background: #fff;
  }
  .nav-status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px 2px;
    font-size: 13px;
  }
  .nav-status.online .dot {
    background: var(--ok);
  }
  .nav-status.offline .dot {
    background: var(--err);
  }
  .nav-version {
    padding: 0 14px 0 32px;
    font-size: 11px;
    color: #64748b;
  }
  /* Quick Support: narrow window, icons only. */
  .app.quick .sidebar {
    width: 64px;
    align-items: center;
  }
  .app.quick .nav-label,
  .app.quick .brand-name {
    display: none;
  }
  .app.quick .brand {
    padding: 4px 0 14px;
  }
  .app.quick .nav-item {
    justify-content: center;
    padding: 10px;
    position: relative;
  }
  .app.quick .nav-item .chat-badge {
    position: absolute;
    top: 2px;
    right: 2px;
  }
  .app.quick .nav-status {
    padding: 8px 0;
  }
  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }
  h1 {
    font-size: 20px;
    margin: 0 0 2px;
  }
  h2 {
    font-size: 15px;
    margin: 0 0 6px;
  }
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px 24px;
    width: 100%;
    max-width: 860px;
    box-sizing: border-box;
  }
  main.chat-page {
    min-height: 0;
  }
  main.chat-page .chat-panel.docked {
    flex: 1;
    max-height: none;
    min-height: 260px;
    margin: 0;
  }
  .row .icon-btn.active {
    border-color: var(--accent);
    color: var(--accent);
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .card.log {
    overflow: auto;
  }
  .label {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
    margin-top: 8px;
    display: block;
  }
  .hint {
    color: var(--muted);
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
  .mono {
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  }
  .value-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warn);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  input {
    width: 100%;
    box-sizing: border-box;
    padding: 9px 11px;
    font-size: 17px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }
  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  button {
    font: inherit;
    padding: 8px 14px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
    margin-top: 10px;
  }
  .icon-btn {
    font-size: 13px;
    padding: 5px 10px;
  }
  .error {
    color: var(--err);
    margin: 4px 0 0;
    font-size: 13px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 13px;
  }
  li {
    padding: 3px 0;
    border-bottom: 1px solid var(--border);
  }
  li.warn {
    color: var(--warn);
  }
  li.error {
    color: var(--err);
  }
  .time {
    color: var(--muted);
    margin-right: 10px;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(2, 6, 23, 0.55);
    display: grid;
    place-items: center;
  }
  .modal {
    background: var(--surface);
    border-radius: 14px;
    padding: 22px 24px;
    width: min(420px, calc(100vw - 32px));
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.35);
  }
  .modal p {
    margin: 8px 0;
  }
  .countdown {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    margin: 14px 0;
    overflow: hidden;
  }
  .countdown div {
    height: 100%;
    background: var(--warn);
    transition: width 0.25s linear;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }
  .actions .primary {
    margin-top: 0;
  }
  button.danger {
    background: var(--err);
    border-color: var(--err);
    color: #fff;
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .muted {
    color: var(--muted);
  }
  .strong {
    font-weight: 600;
  }
  .sharing {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 20px;
    background: color-mix(in srgb, var(--err) 14%, var(--surface));
    border-bottom: 1px solid var(--border);
    font-size: 14px;
  }
  .live {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--err);
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .viewer {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    background: #000;
    --ease-out: cubic-bezier(0.22, 1, 0.36, 1);
    --ease-in-out: cubic-bezier(0.65, 0, 0.35, 1);
    --chat-w: 290px;
  }
  /* ---- Viewer bar, status dot panel and chat drawer: one liquid glass (ADR 0067) ---- */
  .glass {
    background:
      radial-gradient(70% 160% at 18% -40%, rgba(255, 255, 255, 0.16), transparent 60%),
      linear-gradient(180deg, rgba(255, 255, 255, 0.1), rgba(255, 255, 255, 0.02) 55%, rgba(255, 255, 255, 0.05)),
      var(--g-tint);
    border: 1px solid var(--g-edge);
    color: var(--text);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.22), 0 10px 30px var(--g-shadow);
  }
  .glass-note {
    background: var(--g-note);
    border: 1px solid var(--g-sep);
    color: var(--text);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    box-shadow: 0 6px 18px var(--g-shadow);
  }
  .bar-space {
    flex: none;
  }
  .viewer svg.i {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex: none;
  }
  /* Hiding shrinks the bar into the dot's corner: only transform and opacity move. */
  .vbar {
    position: absolute;
    left: 8px;
    right: 8px;
    top: 8px;
    z-index: 6;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 7px 10px 7px 9px;
    border-radius: 18px;
    font-size: 13px;
    backdrop-filter: blur(18px) saturate(1.7);
    -webkit-backdrop-filter: blur(18px) saturate(1.7);
    transform-origin: 15px 15px;
    will-change: transform, opacity;
    transition:
      transform var(--dur-slow) var(--spring-soft),
      opacity 0.3s var(--ease-out),
      right 0.55s var(--ease-in-out);
  }
  .vbar.gone {
    transform: scale(0.03, 0.35);
    opacity: 0;
    pointer-events: none;
    transition:
      transform 0.55s var(--ease-in-out),
      opacity 0.4s var(--ease-out) 0.12s;
  }
  .viewer.chat-open .vbar {
    right: calc(var(--chat-w) + 14px);
  }
  .viewer.chat-open .vbar .lbl {
    display: none;
  }
  /* Pinned chat: the picture (and the mouse mapping, which follows the canvas) fits the space left of it. */
  .viewer.chat-pinned .screen {
    margin-right: calc(var(--chat-w) + 8px);
  }
  .vrow {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .vdot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex: none;
    background: var(--ok);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 22%, transparent);
  }
  .vdot.caution {
    background: var(--caution);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--caution) 24%, transparent);
  }
  .vdot.critical {
    background: var(--err);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--err) 24%, transparent);
  }
  .vname {
    font-weight: 600;
    white-space: nowrap;
  }
  .vid {
    color: var(--g-dim);
    letter-spacing: 0.02em;
  }
  .viewer button {
    font: inherit;
  }
  .vib {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 26px;
    padding: 0;
    border-radius: 8px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--g-dim);
    cursor: pointer;
    transition:
      background 0.3s var(--ease-out),
      color 0.3s var(--ease-out),
      transform 0.45s var(--spring);
  }
  .vib:hover {
    background: var(--g-hover);
    color: var(--text);
  }
  .vib.on {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .vseg {
    display: inline-flex;
    align-items: center;
    padding: 2px;
    gap: 2px;
    border-radius: 12px;
    background: var(--g-recess);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.2);
  }
  .vseg button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 9px;
    border-radius: 9px;
    border: none;
    background: transparent;
    color: var(--g-dim);
    font-size: 12.5px;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background 0.3s var(--ease-out),
      color 0.3s var(--ease-out),
      transform 0.45s var(--spring);
  }
  .vseg button:hover {
    color: var(--text);
    background: var(--g-hover);
  }
  .vseg button.on {
    color: var(--text);
    background: var(--g-on);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.12);
  }
  .vseg button.on svg {
    color: var(--accent);
  }
  .vseg button.unread {
    color: var(--accent);
  }
  .vseg .badge {
    margin: 0 4px;
  }
  .viewer .count {
    font-size: 10px;
    font-weight: 700;
    background: var(--accent);
    color: var(--accent-text);
    border-radius: 999px;
    padding: 0 5px;
  }
  .vselect {
    font: inherit;
    font-size: 12.5px;
    padding: 4px 8px;
    border-radius: 9px;
    border: 1px solid var(--g-sep);
    background: var(--g-recess);
    color: var(--text);
  }
  .vdisc {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 11px;
    border-radius: 9px;
    font-weight: 600;
    font-size: 12.5px;
    cursor: pointer;
    border: 1px solid var(--d-line);
    background: var(--d-bg);
    color: var(--d-text);
    transition:
      background 0.35s var(--ease-out),
      color 0.35s var(--ease-out),
      border-color 0.35s var(--ease-out),
      transform 0.45s var(--spring);
  }
  .vdisc:hover {
    background: var(--err);
    border-color: var(--err);
    color: #fff;
  }
  .vib:active,
  .vseg button:active,
  .vdisc:active,
  .b-act:active,
  .vsend:active {
    transform: scale(0.94);
  }
  .vstats {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 0;
    font-size: 11.5px;
    padding-left: 1px;
  }
  .vstats > span {
    display: inline-flex;
    gap: 5px;
    align-items: baseline;
    padding: 0 10px;
    border-left: 1px solid var(--g-sep);
    white-space: nowrap;
    color: var(--g-dim);
  }
  .vstats > span:first-child {
    border-left: none;
    padding-left: 0;
  }
  .vstats b {
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-weight: 500;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .vstats .caution b {
    color: var(--caution-text);
  }
  .vstats .critical b {
    color: var(--err);
  }
  .vpath {
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
    white-space: nowrap;
    background: color-mix(in srgb, var(--ok) 15%, transparent);
    color: var(--ok-text);
  }
  .vpath.relay {
    background: color-mix(in srgb, var(--caution) 16%, transparent);
    color: var(--caution-text);
  }
  .vrow.transfer progress {
    flex: 1;
    min-width: 120px;
  }
  :is(.vib, .vseg button, .vdisc, .b-act, .chat-tab, .vsend):focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  /* The dot panel: one shape. Closed, its clip is an 18px circle around the dot (6px in from the
     corner); open, the same clip grows into the rounded panel. */
  .blob {
    position: absolute;
    left: 8px;
    top: 8px;
    z-index: 5;
    box-sizing: border-box;
    width: 380px;
    max-width: calc(100vw - 16px);
    height: var(--h, 220px);
    border-radius: 18px;
    outline: none;
    clip-path: inset(6px calc(100% - 24px) calc(100% - 24px) 6px round 9px);
    opacity: 0;
    transform: scale(0.4);
    transform-origin: 15px 15px;
    pointer-events: none;
    transition:
      clip-path var(--dur) var(--spring),
      opacity 0.25s var(--ease-out),
      transform 0.45s var(--spring);
  }
  .blob.shown {
    opacity: 1;
    transform: none;
    pointer-events: auto;
    transition:
      clip-path 0.42s var(--ease-in-out),
      opacity 0.25s var(--ease-out) 0.3s,
      transform 0.45s var(--spring) 0.3s;
  }
  .blob.shown.open {
    clip-path: inset(0 0 0 0 round 18px);
    backdrop-filter: blur(18px) saturate(1.7);
    -webkit-backdrop-filter: blur(18px) saturate(1.7);
    transition: clip-path var(--dur-slow) var(--spring-soft);
  }
  .blob.fading {
    clip-path: inset(0 0 0 0 round 18px);
    opacity: 0;
    transform: scale(1.02);
    pointer-events: none;
    transition:
      opacity 0.35s var(--ease-out),
      transform 0.45s var(--ease-out);
  }
  /* (18 - 10) / 2 = 4 from the circle, which starts 6px in, less the 1px border. */
  .blob > .vdot {
    position: absolute;
    left: 9px;
    top: 9px;
    box-shadow: 0 0 0 4px var(--g-on);
  }
  .blob-body {
    position: absolute;
    left: 0;
    top: 0;
    width: 100%;
    box-sizing: border-box;
    padding: 7px 12px 12px 28px;
    display: flex;
    flex-direction: column;
    gap: 9px;
    font-size: 12.5px;
    opacity: 0;
    filter: blur(6px);
    transform: scale(0.92);
    transform-origin: 15px 15px;
    transition:
      opacity 0.42s var(--ease-in-out),
      filter 0.42s var(--ease-in-out),
      transform 0.42s var(--ease-in-out);
  }
  .blob.open .blob-body {
    opacity: 1;
    filter: blur(0);
    transform: none;
    transition:
      opacity 0.35s var(--ease-out) 0.08s,
      filter 0.35s var(--ease-out) 0.08s,
      transform var(--dur-slow) var(--spring-soft) 0.04s;
  }
  .b-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 16px;
  }
  .b-name {
    color: var(--g-dim);
    font-size: 12px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .b-rest {
    margin-left: -16px;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .b-rest > * {
    opacity: 0;
    transform: translateY(-4px);
    transition:
      opacity 0.42s var(--ease-in-out),
      transform 0.42s var(--ease-in-out);
  }
  .blob.open .b-rest > * {
    opacity: 1;
    transform: none;
    transition:
      opacity 0.35s var(--ease-out) var(--d, 0.1s),
      transform 0.5s var(--spring-soft) var(--d, 0.1s);
  }
  .b-rest > :nth-child(1) {
    --d: 0.14s;
  }
  .b-rest > :nth-child(2) {
    --d: 0.19s;
  }
  .b-rest > :nth-child(3) {
    --d: 0.24s;
  }
  .b-verdict {
    color: var(--ok-text);
  }
  .b-verdict.caution {
    color: var(--caution-text);
  }
  .b-verdict.critical {
    color: var(--err);
  }
  .b-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 3px;
    padding: 3px;
    border-radius: 12px;
    background: var(--g-recess);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.2);
  }
  .b-cell {
    border-radius: 9px;
    padding: 4px 8px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .b-cell small {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--g-dim);
  }
  .b-cell span {
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }
  .b-cell.caution {
    background: color-mix(in srgb, var(--caution) 14%, transparent);
  }
  .b-cell.caution span {
    color: var(--caution-text);
  }
  .b-cell.critical {
    background: color-mix(in srgb, var(--err) 16%, transparent);
  }
  .b-cell.critical span {
    color: var(--err);
  }
  .b-acts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 5px;
  }
  .b-act {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 6px 9px;
    border-radius: 9px;
    border: 1px solid transparent;
    background: var(--g-on);
    color: var(--text);
    font-size: 12.5px;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background 0.3s var(--ease-out),
      color 0.3s var(--ease-out),
      border-color 0.3s var(--ease-out),
      transform 0.45s var(--spring);
  }
  .b-act:hover {
    background: var(--g-hover);
  }
  .b-act.restore svg {
    color: var(--accent);
  }
  .b-act.disc {
    border-color: var(--d-line);
    background: var(--d-bg);
    color: var(--d-text);
    font-weight: 600;
  }
  .b-act.disc:hover {
    background: var(--err);
    border-color: var(--err);
    color: #fff;
  }
  .bar-hint {
    position: absolute;
    left: 42px;
    top: 10px;
    z-index: 4;
    max-width: min(320px, calc(100vw - 60px));
    padding: 7px 11px;
    border-radius: 12px;
    font-size: 12.5px;
    line-height: 1.35;
    opacity: 0;
    transform: translateX(-16px) scale(0.9);
    transform-origin: left center;
    pointer-events: none;
    transition:
      opacity 0.35s var(--ease-out),
      transform var(--dur-slow) var(--spring-soft);
  }
  .bar-hint.show {
    opacity: 1;
    transform: none;
    transition-delay: 0.45s;
  }
  .bar-hint b {
    color: var(--accent);
  }

  /* Chat drawer anchored to the right edge; hides by sliding out, its tab stays on the edge. */
  .vchat {
    position: absolute;
    top: 8px;
    bottom: 8px;
    right: 0;
    width: var(--chat-w);
    z-index: 7;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    border-radius: 18px 0 0 18px;
    border-right: none;
    font-size: 13px;
    backdrop-filter: blur(18px) saturate(1.7);
    -webkit-backdrop-filter: blur(18px) saturate(1.7);
    transform: translateX(100%);
    will-change: transform;
    transition: transform var(--dur-slow) var(--spring-soft);
  }
  .vchat.open {
    transform: none;
  }
  .vchat-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 10px 8px 14px;
    border-bottom: 1px solid var(--g-sep);
  }
  .vchat-log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 10px 6px 12px;
    scrollbar-width: thin;
  }
  .vchat-empty {
    margin: auto 8px;
    text-align: center;
    color: var(--g-dim);
    font-size: 12px;
  }
  .vmsg {
    max-width: 84%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    animation: vmsg-in 0.55s var(--spring-soft) both;
  }
  .vmsg .bubble {
    padding: 6px 10px;
    border-radius: 14px 14px 14px 5px;
    line-height: 1.35;
    background: var(--g-on);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.12);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .vmsg time {
    font-size: 10px;
    color: var(--g-dim);
    padding: 0 6px;
    font-variant-numeric: tabular-nums;
  }
  .vmsg.mine {
    align-self: flex-end;
    align-items: flex-end;
  }
  .vmsg.mine .bubble {
    border-radius: 14px 14px 5px 14px;
    background: var(--accent);
    color: var(--accent-text);
  }
  @keyframes vmsg-in {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.96);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .vchat-compose {
    display: flex;
    flex-direction: row;
    flex-wrap: nowrap;
    gap: 6px;
    align-items: flex-end;
    padding: 8px 10px 10px 12px;
  }
  .vchat-compose textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    height: 36px;
    max-height: 120px;
    width: auto;
    box-sizing: border-box;
    font: inherit;
    line-height: 1.35;
    padding: 8px 11px;
    border-radius: 12px;
    border: 1px solid var(--g-sep);
    background: var(--g-recess);
    color: var(--text);
    outline: none;
    transition: border-color 0.25s var(--ease-out);
  }
  .vchat-compose textarea::placeholder {
    color: var(--g-dim);
  }
  .vchat-compose textarea:focus {
    border-color: var(--accent);
  }
  .vsend {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    border: none;
    background: var(--accent);
    color: var(--accent-text);
    display: grid;
    place-items: center;
    cursor: pointer;
    flex: none;
    transition: transform 0.45s var(--spring);
  }
  .chat-tab {
    position: absolute;
    left: -27px;
    top: 50%;
    width: 27px;
    height: 64px;
    margin-top: -32px;
    touch-action: none;
    box-sizing: border-box;
    padding: 0;
    display: grid;
    place-items: center;
    border-radius: 12px 0 0 12px;
    border: 1px solid var(--g-edge);
    border-right: none;
    color: var(--g-dim);
    background:
      linear-gradient(90deg, rgba(255, 255, 255, 0.1), rgba(255, 255, 255, 0.03)),
      var(--g-tint);
    backdrop-filter: blur(18px) saturate(1.7);
    -webkit-backdrop-filter: blur(18px) saturate(1.7);
    box-shadow: inset 1px 1px 0 rgba(255, 255, 255, 0.2), -6px 0 18px var(--g-shadow);
    cursor: pointer;
    opacity: 0;
    transform: translateX(10px);
    pointer-events: none;
    transition:
      opacity 0.3s var(--ease-out),
      transform 0.5s var(--spring-soft),
      color 0.3s var(--ease-out),
      background 0.3s var(--ease-out);
  }
  .vchat.used:not(.open) .chat-tab {
    opacity: 1;
    transform: none;
    pointer-events: auto;
    transition-delay: 0.15s, 0.15s, 0s, 0s;
  }
  .chat-tab:hover {
    color: var(--text);
  }
  .tab-badge {
    position: absolute;
    top: -7px;
    left: -6px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    box-sizing: border-box;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    display: grid;
    place-items: center;
    background: var(--text);
    color: var(--bg);
    transform: scale(0);
    transition: transform 0.5s var(--spring);
  }
  /* New messages while hidden: the tab fills with the accent colour and breathes. */
  .chat-tab.unread {
    color: var(--accent-text);
    background: var(--accent);
    border-color: var(--accent);
    animation: tab-breathe 2.2s ease-in-out infinite;
  }
  .chat-tab.unread .tab-badge {
    transform: scale(1);
  }
  @keyframes tab-breathe {
    0%,
    100% {
      box-shadow: -6px 0 18px var(--g-shadow), 0 0 0 0 color-mix(in srgb, var(--accent) 55%, transparent);
    }
    50% {
      box-shadow: -6px 0 18px var(--g-shadow), 0 0 0 7px color-mix(in srgb, var(--accent) 0%, transparent);
    }
  }
  .chat-peek {
    position: absolute;
    right: 36px;
    top: 50%;
    margin-top: -22px;
    max-width: 240px;
    z-index: 6;
    pointer-events: none;
    padding: 7px 11px;
    border-radius: 14px 14px 5px 14px;
    font-size: 12.5px;
    line-height: 1.35;
    opacity: 0;
    transform: translateX(14px) scale(0.92);
    transform-origin: right center;
    transition:
      opacity 0.3s var(--ease-out),
      transform var(--dur-slow) var(--spring-soft);
  }
  .chat-peek b {
    display: block;
    font-size: 11px;
    color: var(--g-dim);
    margin-bottom: 1px;
  }
  .chat-peek.show {
    opacity: 1;
    transform: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .viewer * {
      transition: none !important;
      animation: none !important;
    }
  }
  .toolbar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .toolbar-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 12px;
    width: 100%;
  }
  .name-input {
    max-width: 320px;
  }
  /* Overall connection state, always shown: green, yellow (attention), red (critical). */
  .health {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex: none;
    background: var(--ok);
  }
  .health.caution {
    background: var(--caution);
  }
  .health.critical {
    background: var(--err);
  }
  .stat {
    display: inline-flex;
    flex-direction: column;
    gap: 1px;
    padding: 3px 10px;
    border-radius: 7px;
    background: var(--surface);
    border: 1px solid var(--border);
    line-height: 1.25;
  }
  .stat-k {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  /* Minimum width (set per stat) and padded values: the toolbar does not shift as numbers
     change, and a value is never cut (some glyphs, e.g. arrows, are wider than 1ch). */
  .stat-v {
    font-size: 12px;
    color: var(--text);
    white-space: pre;
  }
  .stat.caution {
    border-color: var(--caution);
    box-shadow: 0 0 0 1px var(--caution);
  }
  .stat.critical {
    border-color: var(--err);
    box-shadow: 0 0 0 1px var(--err);
  }
  .screen {
    flex: 1;
    min-height: 0;
    position: relative;
    display: flex;
    overflow: auto;
  }
  /* Sized by the script (`picture`); auto margins centre it, and become 0 when it is larger
     than the window, so the scroll bars reach every edge. */
  .screen canvas {
    flex: none;
    margin: auto;
    width: 100%;
    height: 100%;
  }
  .screen canvas:focus {
    outline: none;
  }
  .screen canvas.controlling {
    cursor: default;
  }
  .displays {
    font: inherit;
    font-size: 13px;
    padding: 4px 8px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  /* Home ("Supporto remoto"): two cards side by side, saved devices below. */
  main.home {
    max-width: 1000px;
  }
  .page-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .page-head h1 {
    font-size: 22px;
    margin: 0;
  }
  .home-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: 16px;
    align-items: start;
  }
  .app.quick .home-grid {
    grid-template-columns: 1fr;
    max-width: 560px;
    width: 100%;
    margin: 0 auto;
  }
  .home-card {
    border-radius: 14px;
    padding: 18px 20px;
    gap: 12px;
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .card-head h2 {
    margin: 0;
  }
  .card-icon {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: 9px;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .card-icon svg,
  .square-btn svg,
  .shield svg,
  .connect-btn svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .credentials {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--bg);
  }
  .cred-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cred-row + .cred-row {
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .cred {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .cred .label {
    margin-top: 0;
    font-size: 11px;
    font-weight: 600;
  }
  .cred-id {
    font-size: 28px;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .cred-pw {
    font-size: 22px;
    font-weight: 600;
    letter-spacing: 0.06em;
  }
  .square-btn {
    width: 34px;
    height: 34px;
    padding: 0;
    flex-shrink: 0;
    display: grid;
    place-items: center;
  }
  .square-btn svg {
    width: 16px;
    height: 16px;
  }
  .unattended-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .shield {
    display: grid;
    color: var(--muted);
  }
  .shield svg {
    width: 20px;
    height: 20px;
  }
  .shield.on {
    color: var(--ok);
  }
  .ok-text {
    color: var(--ok);
  }
  .unattended-title {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  /* On / off label of the unattended access. */
  .state-pill {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
    white-space: nowrap;
    color: var(--muted);
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .state-pill.on {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 12%, transparent);
    border-color: color-mix(in srgb, var(--ok) 30%, transparent);
  }
  .menu-anchor {
    position: relative;
    flex: none;
  }
  .icon-btn.more {
    display: grid;
    place-items: center;
    padding: 5px 8px;
  }
  .icon-btn.primary-small {
    flex: none;
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }
  .more svg,
  .popup-menu svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .popup-menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 20;
    min-width: 220px;
    display: flex;
    flex-direction: column;
    padding: 5px;
    background: var(--raised);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 8px 24px color-mix(in srgb, #000 25%, transparent);
  }
  .popup-menu button {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 8px 10px;
    border: none;
    background: none;
    text-align: left;
    font-size: 13px;
  }
  .popup-menu button:hover {
    background: var(--bg);
  }
  .popup-menu .danger-item {
    color: var(--err);
  }
  .connect-form {
    gap: 10px;
  }
  .connect-form .label {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 600;
  }
  .partner-id {
    font-size: 20px;
    letter-spacing: 0.04em;
  }
  .segmented.wide button {
    flex: 1;
  }
  .connect-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    margin-top: 4px;
    padding: 11px 14px;
    border-radius: 10px;
    font-weight: 700;
  }
  .quick-devices {
    border-radius: 14px;
    padding: 12px 16px;
    gap: 10px;
  }
  .quick-devices-head {
    display: flex;
    align-items: center;
  }
  .quick-devices-head .group-title {
    flex: 1;
    margin: 0;
  }
  .quick-devices-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 10px;
  }
  .quick-device {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    border-radius: 10px;
    background: var(--bg);
    text-align: left;
    min-width: 0;
  }
  .quick-device:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .quick-device-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .quick-device-name {
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot.ok {
    background: var(--ok);
  }
  .dot.off {
    background: var(--muted);
  }
  .prefs-link {
    margin-top: 2px;
  }

  /* Preferences: tabs, then rows with the text on the left and the control on the right. */
  .tabs {
    display: flex;
    gap: 4px;
    margin: 6px 0 10px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: transparent;
    color: var(--muted);
    font-size: 14px;
    white-space: nowrap;
  }
  .tabs button:hover {
    color: var(--text);
  }
  .tabs button.on {
    border-bottom-color: var(--accent);
    color: var(--text);
    font-weight: 600;
  }
  .tabs button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .pref.column {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }
  .size-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
    gap: 8px;
  }
  .size-tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 6px 10px;
    border-radius: 10px;
    background: var(--bg);
  }
  .size-tile:hover {
    border-color: var(--muted);
  }
  .size-tile.on {
    border: 2px solid var(--accent);
    padding: 11px 5px 9px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .size-tile .hint {
    text-align: center;
  }
  .size-shape {
    display: grid;
    place-items: center;
    width: 64px;
    height: 42px;
  }
  .size-shape span {
    box-sizing: border-box;
    border: 2px solid var(--muted);
    border-radius: 3px;
  }
  .size-tile.on .size-shape span {
    border-color: var(--accent);
  }
  .size-label {
    font-size: 13px;
    font-weight: 600;
  }
  main.prefs {
    gap: 8px;
    max-width: 800px;
  }
  main.prefs > .hint {
    margin-bottom: 6px;
  }
  .group-title {
    margin: 12px 0 2px 4px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
  }
  .group-note {
    margin: 0 4px;
  }
  .card.rows {
    padding: 0;
    gap: 0;
    overflow: hidden;
  }
  .card.rows.highlight {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .card.rows .notes {
    margin: 0 18px 14px;
  }
  .pref {
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 14px 18px;
  }
  .pref + .pref {
    border-top: 1px solid var(--border);
  }
  .pref-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pref-title {
    font-size: 14px;
    font-weight: 600;
  }
  .pref-text .error {
    margin: 2px 0 0;
    font-size: 12px;
  }
  .pref-text .countdown {
    margin: 6px 0;
  }
  .pref .primary {
    margin-top: 0;
  }
  .pref .value-row {
    flex-shrink: 0;
    max-width: 360px;
  }
  .pref .name-input {
    font-size: 14px;
    padding: 6px 9px;
  }
  .pref-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .pill {
    margin-left: 6px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
    vertical-align: 1px;
  }
  .status-line {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dot.ok {
    width: 7px;
    height: 7px;
    background: var(--ok);
  }
  .switch {
    flex-shrink: 0;
    width: 40px;
    height: 24px;
    padding: 2px;
    border: none;
    border-radius: 12px;
    background: var(--switch-off);
    display: flex;
    transition: background 0.15s;
  }
  .switch span {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: transform 0.15s;
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
  }
  .switch[aria-checked="true"] span {
    transform: translateX(16px);
  }
  .switch:focus-visible,
  .segmented button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .segmented {
    display: flex;
    flex-shrink: 0;
    gap: 2px;
    padding: 3px;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: var(--bg);
  }
  .segmented button {
    font-size: 13px;
    padding: 5px 14px;
    border: none;
    border-radius: 7px;
    background: transparent;
    color: var(--muted);
  }
  .segmented button.on {
    background: var(--raised);
    color: var(--text);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  button.link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-size: inherit;
    text-decoration: underline;
  }
  .notes {
    font-size: 12px;
    max-height: 220px;
    overflow: auto;
    background: var(--bg);
    border-radius: 8px;
    padding: 8px;
  }
  .notes pre {
    white-space: pre-wrap;
    font: inherit;
    margin: 4px 0 10px;
  }
  .connecting {
    align-items: center;
    margin-top: 10px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 14px;
  }
  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .permissions {
    margin-top: 10px;
    padding: 10px 12px;
    border: 1px solid var(--warn);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
  }
  .permissions strong {
    color: var(--warn);
  }
  .perm-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    margin-top: 8px;
    cursor: pointer;
  }
  .check input {
    width: auto;
  }
  .unattended {
    margin: 0;
    font-size: 13px;
  }
  .unattended.on {
    color: var(--ok);
  }
  .modal.ended {
    min-width: 360px;
  }
  .modal.ended h2 {
    margin-bottom: 16px;
  }
  .ended-duration {
    display: flex;
    flex-direction: column;
    gap: 2px;
    align-items: center;
    padding: 16px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .ended-duration-label {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  .ended-duration strong {
    font-size: 28px;
    color: var(--accent);
  }
  .ended-rows {
    margin-top: 14px;
    display: flex;
    flex-direction: column;
  }
  .ended-row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 16px;
    padding: 8px 2px;
    border-top: 1px solid var(--border);
  }
  .lan-name {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .presence {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 7ch;
    font-size: 12px;
    color: var(--muted);
  }
  .presence::before {
    content: "";
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.6;
  }
  .presence.on {
    color: var(--ok);
  }
  .presence.on::before {
    opacity: 1;
  }
  .devices li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
  }
  .card-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .tree li {
    padding-left: calc(var(--depth, 0) * 22px);
  }
  .tree li.folder {
    border-top: 1px solid var(--border);
  }
  .tree li.empty {
    padding-left: calc(var(--depth, 0) * 22px + 22px);
  }
  .tree li.picked {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .pick,
  .choice input {
    width: auto;
    flex: none;
    margin: 0 2px 0 4px;
    padding: 0;
  }
  .row-actions {
    display: flex;
    gap: 8px;
  }
  .row-actions .primary {
    margin-top: 0;
  }
  .row-actions {
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .hidden-file {
    display: none;
  }
  .modal input[type="password"] + input[type="password"] {
    margin-top: 8px;
  }
  .undo-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    margin: 6px 0;
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface));
  }
  .folder-choices {
    max-height: 300px;
    overflow-y: auto;
    margin: 10px 0 14px;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px 0;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    white-space: pre;
    cursor: pointer;
    font-size: 14px;
  }
  .choice:hover {
    background: var(--bg);
  }
  .folder-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border: none;
    background: none;
    min-width: 0;
    text-align: left;
  }
  .folder-toggle:hover {
    background: var(--surface);
  }
  .chevron {
    transition: transform 0.15s ease;
    flex: none;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  select {
    font: inherit;
    font-size: 13px;
    padding: 5px 8px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    max-width: 180px;
  }
  .modal select {
    width: 100%;
    max-width: none;
    margin-top: 4px;
  }
  .modal input {
    margin-top: 8px;
  }
  .limited {
    color: var(--warn);
  }
  button.attention {
    animation: attention 1.6s ease-in-out infinite;
  }
  @keyframes attention {
    0%,
    100% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 60%, transparent);
    }
    50% {
      box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 0%, transparent);
    }
  }
  .usage {
    white-space: nowrap;
  }
  .badge {
    font-size: 12px;
    padding: 3px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .badge.path.lan,
  .badge.path.direct {
    color: var(--ok);
    border-color: var(--ok);
  }
  .badge.path.relay {
    color: var(--accent);
    border-color: var(--accent);
  }
  .badge.warn {
    color: var(--warn);
    border-color: var(--warn);
  }
  .chat-badge {
    display: inline-block;
    min-width: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent);
    color: #1a1205;
    font-size: 11px;
    font-weight: 700;
    text-align: center;
  }
  .popup-chat {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
  }
  .popup-chat .chat-log {
    flex: 1;
  }
  .cancel-transfer {
    padding: 1px 8px;
    line-height: 1.2;
  }
  .volume {
    width: 90px;
    accent-color: var(--accent);
  }
  .banner-progress {
    width: 180px;
    accent-color: var(--accent);
  }
  .chat-panel {
    position: absolute;
    right: 12px;
    bottom: 12px;
    width: 320px;
    max-height: 60%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }
  .chat-panel.docked {
    position: static;
    width: 100%;
    max-height: 240px;
    margin: 8px 0 0;
  }
  .chat-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  .chat-log {
    flex: 1;
    overflow-y: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chat-msg {
    max-width: 85%;
    align-self: flex-start;
    padding: 6px 10px;
    border-radius: 10px;
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .chat-msg.mine {
    align-self: flex-end;
    background: color-mix(in srgb, var(--accent) 22%, var(--surface));
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .chat-text {
    display: block;
    font-size: 13px;
    word-wrap: break-word;
    white-space: pre-wrap;
  }
  .chat-time {
    display: block;
    text-align: right;
    font-size: 10px;
    color: var(--muted);
  }
  .chat-input {
    display: flex;
    gap: 6px;
    padding: 8px;
    border-top: 1px solid var(--border);
  }
  .chat-input input {
    flex: 1;
    min-width: 0;
  }
  .notice {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    margin: 0;
    padding: 8px 14px;
    border-radius: 8px;
    background: rgba(20, 20, 20, 0.85);
    color: #fff;
    font-size: 13px;
    pointer-events: none;
  }
  .overlay {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    max-width: 80%;
    text-align: center;
    color: #cbd5e1;
    margin: 0;
  }
  .error-text {
    color: var(--err);
  }
  /* Connecting again: the old picture is hidden, the message is what matters. */
  .reconnect-cover {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: #000;
  }
  .reconnect-cover p {
    margin: 0;
    color: #f1f5f9;
    font-size: 1.25rem;
    text-align: center;
  }
</style>
