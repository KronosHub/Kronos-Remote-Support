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
    showChat: boolean;
    unread: number;
    /** Why the session ended; the window stays open to show it. */
    ended: string | null;
    /** Connecting again by itself (see `Reconnect` in service.rs): the window closes when
     * the new session to the same device opens. */
    reconnecting: boolean;
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
    if (section === "devices") refreshPresence();
  });
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
  type Section = "support" | "devices" | "chat" | "activity" | "updates";
  let section = $state<Section>("support");
  /** Stroke icons (24×24, lucide style) for the sidebar. */
  const icons: Record<Section, string> = {
    support: '<path d="M8 3 4 7l4 4"/><path d="M4 7h16"/><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/>',
    devices: '<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>',
    chat: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
    activity: '<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>',
    updates: '<path d="M21 12a9 9 0 1 1-2.64-6.36L21 8"/><path d="M21 3v5h-5"/>',
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
  let virtualMonitorError = $state("");
  let update = $state<UpdateState>({ kind: "idle" });
  let autostart = $state(false);
  /** Our user name as partners see it, and the text being edited. */
  let displayName = $state<string | null>(null);
  let nameDraft = $state("");
  let usePersonal = $state(false);
  let saveAs = $state("");
  /** Personal password dialog (set / change). */
  let personalDialog = $state<{ first: string; second: string; error: string } | null>(null);
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
        if (view?.session === event.session) receiveChat(view, event.text);
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

  /** Opening the section checks again (an update already found stays shown). */
  function openUpdates() {
    section = "updates";
    if (update.kind !== "available") checkUpdate();
    invoke<VirtualMonitor>("virtual_monitor_status").then((v) => (virtualMonitor = v));
  }

  /** Installs/updates (`install`) or removes the virtual monitor; Windows asks for admin rights. */
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
      chat: [],
      showChat: false,
      unread: 0,
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
  <div class="viewer">
    <div class="toolbar">
      <div class="toolbar-row">
        {#if view.peerName}
          <span class="strong peer-name" title="Nome impostato dal partner">{view.peerName}</span>
          <span class="muted">·</span>
        {/if}
        <span class="mono strong peer">{view.peer}</span>
        <span class="spacer"></span>
        {#if view.displays.length > 1}
          <select
            class="displays"
            value={view.currentDisplay}
            onchange={(e) => selectDisplay(Number(e.currentTarget.value))}
            title="Schermo del partner da visualizzare"
          >
            {#each view.displays as d (d.id)}
              <option value={d.id}>
                {d.name} · {d.width}×{d.height}{d.primary ? " · principale" : ""}
              </option>
            {/each}
          </select>
        {/if}
        {#if view.clipboard !== null && !view.ended}
          <button class:active={view.clipboard} onclick={toggleClipboard}
            title="Testo e file copiati su un computer si possono incollare sull'altro">
            Appunti {view.clipboard ? "condivisi" : "spenti"}
          </button>
        {/if}
        {#if view.ended && view.reconnecting}
          <span class="badge warn">Ricollegamento…</span>
        {:else if view.ended}
          <span class="badge warn">Sessione terminata</span>
        {:else if view.controlError}
          <span class="badge warn" title={view.controlError}>Solo visione</span>
        {:else}
          <button class:active={view.control} onclick={toggleControl}
            title="Invia mouse e tastiera al partner">
            Controllo {view.control ? "attivo" : "spento"}
          </button>
        {/if}
        {#if !view.ended && view.control && !view.controlError}
          <button
            class:active={view.inputBlocked}
            onclick={toggleBlockInput}
            title={view.inputBlocked
              ? "Mouse e tastiera del partner bloccati: chi è davanti al suo computer non può usarli (può sbloccarli premendo Esc tre volte). Clic per sbloccarli"
              : "Blocca mouse e tastiera del partner, così chi è davanti al suo computer non interferisce"}>
            {view.inputBlocked ? "Input partner bloccato" : "Blocca input partner"}
          </button>
        {/if}
        {#if !view.ended}
          <button
            class:active={view.showChat}
            class:blink={view.unread > 0 && !view.showChat}
            onclick={() => view && toggleChat(view)}
            title="Scrivi al partner">
            Chat{#if view.unread > 0} <span class="chat-badge">{view.unread}</span>{/if}
          </button>
        {/if}
        {#if !view.ended}
          <button onclick={openFiles}
            title="Sfoglia le cartelle del partner e copia file nei due sensi">Trasferimento file</button>
        {/if}
        {#if !view.ended && audioSupported}
          <button
            class:active={view.audio}
            onclick={toggleAudio}
            title={view.audio ? "Audio del partner attivo. Clic per spegnerlo" : "Ascolta l'audio del partner (circa 64 kbit/s)"}
            >Audio</button>
          {#if view.audio}
            <input
              class="volume"
              type="range"
              min="0"
              max="100"
              value={audioVolume}
              oninput={(e) => setVolume(Number(e.currentTarget.value))}
              title="Volume: {audioVolume} %"
              aria-label="Volume dell'audio del partner"
            />
          {/if}
        {/if}
        {#if !view.ended}
          <button
            class:active={view.hd}
            onclick={toggleHd}
            title={view.hd
              ? "HD attivo: con la rete buona fino a 2560×1600. Clic per limitare a 1080p (meno banda e CPU)"
              : "HD spento: al massimo 1080p. Clic per riattivarlo"}>HD</button>
        {/if}
        <select
          class="displays"
          value={shownZoom}
          onchange={(e) => setZoom(e.currentTarget.value as Zoom)}
          title="Dimensione dell'immagine del partner. Adatta: riempie la finestra, ingrandita con un filtro che mantiene il testo nitido. Ingrandimento intero: ogni pixel del partner diventa un quadrato esatto di pixel del tuo schermo (il più grande che entra). Pixel reali: un pixel del partner per ogni pixel del tuo schermo, con le barre di scorrimento se non entra (senza controllo la rotellina scorre). Risoluzione adattata (solo computer usati da remoto): il partner porta il suo schermo alla risoluzione più vicina alla finestra, poi la ripristina alla fine"
          aria-label="Dimensione dell'immagine"
        >
          <option value="fit">Adatta</option>
          <option value="whole">Intero ×{wholeMultiple}</option>
          <option value="native">Pixel reali</option>
          {#if view.resizable}
            <option value="remote">Risoluzione adattata</option>
          {/if}
        </select>
        <button class="danger" onclick={disconnect}>{view.ended ? "Chiudi" : "Disconnetti"}</button>
      </div>
      <div class="toolbar-row stats">
        <span class="health {connectionLevel(view).level}" title={connectionLevel(view).reasons.join("\n") ||
            "Connessione in buone condizioni"}></span>
        {#if view.path}
          <span class="badge path {view.path}" title={pathHint[view.path]}>{pathLabel[view.path]}</span>
        {/if}
        {#if view.width}
          <span class="stat {level(view.rtt, 150, 300)}"
            title={"Risoluzione, compressione, fotogrammi al secondo e latenza (giallo da 150 ms, rosso da 300 ms)" +
              (view.hardware === null ? "" : view.hardware
                ? "\nGPU: il video è compresso dal chip video del computer controllato"
                : "\nCPU: il video è compresso dal processore del computer controllato (chip video non disponibile)")}>
            <span class="stat-k">Video</span>
            <span class="stat-v mono" style="min-width: 34ch">{pad(`${view.width}×${view.height}`, 9)} {view.hardware === null ? "   " : view.hardware ? "GPU" : "CPU"} · {pad(view.fps === null ? "–" : view.fps.toFixed(0), 2)} fps · {pad(view.rtt === null ? "–" : view.rtt.toFixed(0), 4)} ms</span>
          </span>
        {/if}
        {#if view.net}
          <span class="stat" title="Traffico della sessione: ricevuto ↓ / inviato ↑">
            <span class="stat-k">Rete</span>
            <span class="stat-v mono" style="min-width: 29ch">↓ {pad(rate(view.net.rx), 11)} · ↑ {pad(rate(view.net.tx), 11)}</span>
          </span>
        {/if}
        {#if view.local || view.partner}
          <span
            class="stat {worst(
              level(view.local?.cpu, 50, 80),
              level(view.partner?.cpu, 50, 80),
              level(view.local?.ram, 800, 1500),
              level(view.partner?.ram, 800, 1500),
            )}"
            title="Risorse usate dall'app: tu / partner (cattura e codifica avvengono sul partner). Giallo da 50 % di CPU o 800 MB, rosso da 80 % o 1,5 GB">
            <span class="stat-k">Risorse</span>
            <span class="stat-v mono" style="min-width: 42ch">Tu {view.local ? `${pad(pct(view.local.cpu), 4)} · ${pad(mb(view.local.ram), 7)}` : pad("–", 14)} · Partner {view.partner ? `${pad(pct(view.partner.cpu), 4)} · ${pad(mb(view.partner.ram), 7)}` : pad("–", 14)}</span>
          </span>
        {/if}
        {#if view.limited}
          <span class="stat {view.limited.kbps < 450 ? 'critical' : 'caution'}"
            title="Banda insufficiente: il partner ha ridotto qualità e fotogrammi ({view.limited.fps} fps max) e, sotto 0,8 Mbit/s, la risoluzione. Torna al massimo quando la rete migliora.">
            <span class="stat-k">Qualità ridotta</span>
            <span class="stat-v mono" style="min-width: 15ch">{pad(rate(view.limited.kbps), 11)}</span>
          </span>
        {/if}
        {#if recentResyncs(view) > 0}
          {@const count = recentResyncs(view)}
          <span class="stat {level(count, 2, 5)}"
            title="Volte nell'ultimo minuto in cui l'immagine è stata ricostruita da capo perché si erano persi dati in rete (giallo da 2, rosso da 5)">
            <span class="stat-k">Resync per minuto</span>
            <span class="stat-v mono" style="min-width: 3ch">{pad(count, 3)}</span>
          </span>
        {/if}
      </div>
      {#if view.transfer}
        <div class="toolbar-row transfer">
          <progress max={view.transfer.total || 1} value={view.transfer.done}></progress>
          <span class="mono small">{transferText(view.transfer)}</span>
          <button class="icon-btn cancel-transfer" onclick={() => view && cancelTransfer(view.session)}
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
        <p class="overlay">{view.ended}: ricollegamento in corso…</p>
      {:else if view.ended}
        <p class="overlay">Sessione terminata: {view.ended}</p>
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
      {#if view.showChat}
        <div class="chat-panel">
          <div class="chat-head">
            <span>Chat con <span class="mono">{view.peer}</span></span>
            <button class="icon-btn" onclick={() => view && toggleChat(view)}>Chiudi</button>
          </div>
          <div class="chat-log">
            {#each view.chat as m}
              <div class="chat-msg" class:mine={m.mine}>
                <span class="chat-text">{m.text}</span>
                <span class="chat-time">{m.time}</span>
              </div>
            {/each}
            {#if view.chat.length === 0}<p class="hint small">Nessun messaggio.</p>{/if}
          </div>
          <form class="chat-input" onsubmit={(e) => view && sendChat(view, e)}>
            <input placeholder="Scrivi un messaggio…" bind:value={chatDraft} maxlength="2000" />
            <button class="primary" type="submit">Invia</button>
          </form>
        </div>
      {/if}
    </div>
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
        <button class="nav-item" class:selected={section === "updates"} onclick={openUpdates}
          title="Versione {appVersion}">
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html icons.updates}</svg>
          <span class="nav-label">Aggiornamenti</span>
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
  <main class="grid">
    <section class="card">
      <h2>Consenti il controllo</h2>
      <p class="hint">Comunica ID e password a chi deve connettersi a questo computer.</p>

      <span class="label">Il tuo ID</span>
      <div class="value-row">
        <span class="big mono">{status.id ?? "— — —"}</span>
        <button class="icon-btn" onclick={() => copy(status.id, "id")} disabled={!status.id}>
          {copied === "id" ? "Copiato" : "Copia"}
        </button>
      </div>

      <span class="label">Password</span>
      <div class="value-row">
        <span class="big mono">{status.password}</span>
        <button class="icon-btn" onclick={() => copy(status.password, "pw")}>
          {copied === "pw" ? "Copiato" : "Copia"}
        </button>
        <button class="icon-btn" onclick={newPassword} title="Genera una nuova password">Nuova</button>
      </div>
      <p class="hint small">Con questa password ogni connessione va confermata da te.</p>

      <span class="label">Il tuo nome</span>
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
      <p class="hint small">Lo vede chi ti controlla e chi controlli, accanto al tuo ID.</p>

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
      <span class="label">Accesso non presidiato</span>
      {#if status.unattended}
        <p class="unattended on">
          Attivo: chi conosce la tua <strong>password personale</strong> entra senza conferma.
        </p>
        <div class="row">
          <button class="icon-btn" onclick={() => (personalDialog = { first: "", second: "", error: "" })}>
            Cambia password personale
          </button>
          <button class="icon-btn" onclick={disablePersonal}>Disattiva</button>
        </div>
        {#if (uac?.supported || isMac) && !quick}
          <label class="check" title={isMac
            ? "Per Mac raggiunti solo da remoto, senza nessuno davanti a un monitor: chi controlla può portare lo schermo alla risoluzione della sua finestra"
            : "Per server e macchine virtuali raggiunti solo da remoto, senza nessuno davanti a un monitor: la sessione di Desktop remoto chiusa resta condivisibile e chi controlla può portare lo schermo alla risoluzione della sua finestra"}>
            <input type="checkbox" checked={remoteOnly} onchange={() => setRemoteOnly(!remoteOnly)} />
            Computer usato solo da remoto (senza monitor)
          </label>
          {#if remoteOnly && uac?.supported && !uac.active}
            <p class="hint small">Serve il controllo finestre di amministratore (UAC) attivo.</p>
          {:else if uac?.supported}
            <p class="hint small">Vale per tutto il computer: cambiarla chiede il permesso di amministratore.</p>
          {/if}
        {/if}
      {:else}
        <p class="hint small">
          Imposta una password personale per controllare questo computer senza conferma{#if quick}, finché
          questa finestra è aperta{/if}.
        </p>
        <div class="row">
          <button class="icon-btn" disabled={!status.id}
            onclick={() => (personalDialog = { first: "", second: "", error: "" })}>
            Imposta password personale
          </button>
        </div>
      {/if}
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
      {#if !quick}
      <label class="check">
        <input type="checkbox" checked={autostart} onchange={toggleAutostart} />
        Avvia all'accesso (nascosto nella barra)
      </label>
      {#if status.unattended && !autostart}
        <p class="hint small">
          Con l'accesso non presidiato conviene attivarlo: dopo l'accesso a Windows (o un riavvio) il computer
          resta raggiungibile e chi ti assiste si ricollega da solo.
        </p>
      {/if}
      {/if}
    </section>

    {#if !quick}
    <section class="card">
      <h2>Controlla un computer remoto</h2>
      <form onsubmit={connect}>
        <label>
          <span class="label">ID partner</span>
          <input
            class="mono"
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
        <label class="check">
          <input type="checkbox" bind:checked={usePersonal} />
          Password personale (senza conferma)
        </label>
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
            class="primary"
            type="submit"
            disabled={status.link !== "online" || partnerId.length < 11 || !partnerPassword}
          >
            Connetti
          </button>
        {/if}
      </form>

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

  {:else if section === "updates"}
  <main>
    <h1>Aggiornamenti</h1>
    <section class="card">
      <p>Versione installata: <strong>{appVersion}</strong></p>
      {#if update.kind === "available"}
        <p>
          È disponibile la versione <strong>{update.info.version}</strong>.
        </p>
        {#if update.info.notes}
          <div class="notes">
            {#each changesSince(update.info.notes, update.info.current) as change}
              {#if change.title}<strong>Versione {change.title}</strong>{/if}
              <pre>{change.body}</pre>
            {/each}
          </div>
        {/if}
        <p class="hint small">
          L'app si riavvia al termine. Su macOS potrebbe servire concedere di nuovo i permessi
          Registrazione schermo e Accessibilità.
        </p>
        <div class="row">
          <button class="primary" onclick={installUpdate}>Installa e riavvia</button>
        </div>
      {:else if update.kind === "installing"}
        <p>Download della versione {update.info.version}…</p>
        <div class="countdown">
          <div
            style="width: {update.total ? Math.min(100, (update.downloaded / update.total) * 100) : 5}%"
          ></div>
        </div>
        <p class="hint small">
          {(update.downloaded / 1048576).toFixed(1)} MB{update.total
            ? ` di ${(update.total / 1048576).toFixed(1)} MB`
            : ""}
        </p>
      {:else}
        {#if update.kind === "none"}
          <p class="hint">Hai già l'ultima versione.</p>
        {:else if update.kind === "error"}
          <p class="error">{update.message}</p>
        {:else if update.kind === "checking"}
          <p class="hint">Controllo in corso…</p>
        {/if}
        <div class="row">
          <button class="icon-btn" disabled={update.kind === "checking"} onclick={checkUpdate}>
            Controlla aggiornamenti
          </button>
        </div>
      {/if}
    </section>
    {#if virtualMonitor?.supported}
      <section class="card">
        <h2>Monitor virtuale</h2>
        <p class="hint">
          Per un PC senza monitor collegato: aggiunge uno schermo virtuale 1920×1080, così chi si
          collega vede e controlla il desktop. Usa Virtual Display Driver (licenza MIT, firmato da
          SignPath Foundation); Windows chiede il permesso di amministratore.
        </p>
        {#if virtualMonitor.installed}
          <p>
            Installato{virtualMonitor.version ? ` (versione ${virtualMonitor.version})` : ""}.
          </p>
          {#if virtualMonitor.bundled && virtualMonitor.version !== virtualMonitor.bundled}
            <p>È disponibile la versione <strong>{virtualMonitor.bundled}</strong>.</p>
          {/if}
        {:else}
          <p>Non installato.</p>
        {/if}
        {#if virtualMonitor.message}<p class="hint small">{virtualMonitor.message}</p>{/if}
        {#if virtualMonitorError}<p class="error">{virtualMonitorError}</p>{/if}
        <div class="row">
          {#if !virtualMonitor.installed}
            <button class="primary" disabled={virtualMonitorBusy} onclick={() => setVirtualMonitor(true)}>
              Installa monitor virtuale
            </button>
          {:else}
            {#if virtualMonitor.bundled && virtualMonitor.version !== virtualMonitor.bundled}
              <button class="primary" disabled={virtualMonitorBusy} onclick={() => setVirtualMonitor(true)}>
                Aggiorna
              </button>
            {/if}
            <button class="danger" disabled={virtualMonitorBusy} onclick={() => setVirtualMonitor(false)}>
              Rimuovi
            </button>
          {/if}
          {#if virtualMonitorBusy}<span class="spinner"></span>{/if}
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
          portare il suo schermo alla risoluzione della propria finestra (ripristinata alla fine).
        </p>
        <p class="hint">
          Rispondi <strong>No</strong> per un Mac con un monitor: chi è davanti vedrebbe cambiare la risoluzione.
          Si può cambiare in seguito da Accesso non presidiato.
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
          davanti a chiunque sia lì. Si può cambiare in seguito da Accesso non presidiato.
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

<style>
  :global(:root) {
    --bg: #f1f5f9;
    --surface: #ffffff;
    --text: #0f172a;
    --muted: #64748b;
    --border: #e2e8f0;
    --accent: #e8830c;
    --accent-2: #f39200;
    --accent-text: #ffffff;
    --ok: #16a34a;
    --warn: #d97706;
    --caution: #eab308;
    --err: #dc2626;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif;
    font-size: 15px;
    color: var(--text);
    background: var(--bg);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --bg: #0b1120;
      --surface: #111827;
      --text: #e5e7eb;
      --muted: #94a3b8;
      --border: #1f2937;
      --accent: #f39200;
      --accent-2: #ffb347;
    }
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
  main.grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    align-content: start;
    max-width: none;
  }
  .app.quick main.grid {
    grid-template-columns: 1fr;
    max-width: 560px;
    margin: 0 auto;
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
  .big {
    font-size: 28px;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .value-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .value-row .big {
    flex: 1;
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
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    background: #000;
  }
  .toolbar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .toolbar .peer {
    font-size: 15px;
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
  .toolbar .peer-name {
    font-size: 15px;
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
  .stat.critical .stat-v {
    color: var(--err);
  }
  .toolbar button {
    padding: 5px 12px;
    font-size: 13px;
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
  .toolbar button.active {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
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
  .toolbar-row.transfer progress,
  .banner-progress {
    width: 180px;
    accent-color: var(--accent);
  }
  .cancel-transfer {
    padding: 1px 8px;
    line-height: 1.2;
  }
  .volume {
    width: 90px;
    accent-color: var(--accent);
  }
  /* New chat message while the chat is closed: the button pulses. */
  button.blink {
    animation: blink 1s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      background: var(--accent);
      border-color: var(--accent);
      color: var(--accent-text);
    }
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
    color: #cbd5e1;
    margin: 0;
  }
  .error-text {
    color: var(--err);
  }
</style>
