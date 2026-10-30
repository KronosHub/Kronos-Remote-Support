// Signaling for Kronos Remote Support on Cloudflare (ADR 0035): the same JSON protocol as
// crates/signaling-proto, served by one Durable Object that holds every
// WebSocket. It only introduces peers and relays the WebRTC handshake; screen, input and
// passwords never pass through it (end-to-end encrypted between the two PCs).
//
// Relay (TURN), optional, as secrets (`wrangler secret put`), either:
// - TURN_URL, TURN_USERNAME, TURN_PASSWORD: a provider with fixed credentials (e.g. the
//   ExpressTURN free plan); TURN_URL may list several urls separated by spaces;
// - TURN_KEY_ID, TURN_KEY_API_TOKEN: a Cloudflare Realtime TURN key (short-lived credentials).
// Credentials only reach registered peers when a session starts, never the app binary.
// Fixed credentials can still be read by anyone who runs a session, so the Cloudflare key
// (credentials that expire) is the safer choice; TURN hand-outs are rate limited per IP.
//
// Abuse limits: every counter lives in memory in the single "global" object (all sockets
// meet there), keyed by client IP (`CF-Connecting-IP`, IPv6 grouped by /64). Counters reset
// when the object hibernates, which only happens while nobody is sending anything. Values
// are far above what the app does: a device keeps one socket and a session relays an
// offer, an answer and a few dozen ICE candidates.

import { DurableObject } from "cloudflare:workers";

/** Largest accepted client message (SDP offers are a few KiB). */
const MAX_MESSAGE = 64 * 1024;
/** Open sockets per IP: generous for an office of PCs behind one NAT. */
const MAX_SOCKETS_PER_IP = 100;
/** New sockets per IP per minute (only accepted ones count, so a 3 s retry loop gets in). */
const SOCKETS_PER_IP_PER_MINUTE = 120;
/** A socket that has not registered by then is dropped when its IP opens another one. */
const REGISTER_TIMEOUT_MS = 30_000;
/** Message rate per socket: a burst of BURST messages, then RATE per second sustained. */
const MESSAGE_BURST = 100;
const MESSAGE_RATE = 5;
/** New device ids per IP per hour (each one is a row kept forever). */
const NEW_IDS_PER_IP_PER_HOUR = 30;
/** `connect` requests per minute: per socket, per IP, per IP+target, per target. */
const CONNECTS_PER_SOCKET = 10;
const CONNECTS_PER_IP = 30;
const CONNECTS_PER_IP_TARGET = 10;
const CONNECTS_PER_TARGET = 60;
/** Sessions that hand out TURN credentials, per caller IP per hour; then STUN only. */
const TURN_SESSIONS_PER_IP_PER_HOUR = 30;
/** Open sessions started by one device; older ones are forgotten beyond this. */
const MAX_SESSIONS_PER_PEER = 32;
/** Upper bound on the keys of each rate-limit table (memory cap against many IPs). */
const MAX_LIMITER_KEYS = 50_000;
// Presence (which saved devices are online): ids per request, least ms between requests of
// one connection. Enough for a device list, far too slow to scan ids.
const MAX_PRESENCE_IDS = 20;
const PRESENCE_INTERVAL_MS = 10_000;
/** WebSocket close code for policy violations (rate limits). */
const CLOSE_POLICY = 1008;
const CLOSE_TOO_BIG = 1009;
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const STUN = { urls: ["stun:stun.cloudflare.com:3478"] };
// TURN credentials live for the longest plausible session.
const TURN_TTL = 12 * 3600;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/health") return new Response("ok");
    if (url.pathname !== "/ws") return new Response("not found", { status: 404 });
    if (request.headers.get("Upgrade") !== "websocket") {
      return new Response("expected a websocket", { status: 426 });
    }
    return env.HUB.get(env.HUB.idFromName("global")).fetch(request);
  },
};

export class Hub extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    // In-memory abuse counters (see the top of the file); bounded, swept as they expire.
    this.socketsPerIp = new Limiter(SOCKETS_PER_IP_PER_MINUTE, MINUTE);
    this.newIdsPerIp = new Limiter(NEW_IDS_PER_IP_PER_HOUR, HOUR);
    this.connectsPerSocket = new Limiter(CONNECTS_PER_SOCKET, MINUTE);
    this.connectsPerIp = new Limiter(CONNECTS_PER_IP, MINUTE);
    this.connectsPerIpTarget = new Limiter(CONNECTS_PER_IP_TARGET, MINUTE);
    this.connectsPerTarget = new Limiter(CONNECTS_PER_TARGET, MINUTE);
    this.turnPerIp = new Limiter(TURN_SESSIONS_PER_IP_PER_HOUR, HOUR);
    // socket conn -> { tokens, at }: message token bucket.
    this.buckets = new Map();
    // device key hash -> stable 9-digit id (survives restarts, like the old ids.json).
    this.sql.exec("CREATE TABLE IF NOT EXISTS ids (key_hash TEXT PRIMARY KEY, id TEXT UNIQUE NOT NULL)");
    this.sql.exec("CREATE TABLE IF NOT EXISTS sessions (session INTEGER PRIMARY KEY, a TEXT NOT NULL, b TEXT NOT NULL)");
    this.sql.exec("CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v INTEGER NOT NULL)");
  }

  async fetch(request) {
    const ip = ipKey(request.headers.get("CF-Connecting-IP"));
    const tag = `ip:${ip}`;
    const now = Date.now();
    // Sockets of this IP that never registered are dropped first.
    let open = this.ctx.getWebSockets(tag);
    for (const s of open) {
      const a = s.deserializeAttachment();
      if (a && !a.id && now - (a.since ?? 0) > REGISTER_TIMEOUT_MS) {
        try {
          s.close(CLOSE_POLICY, "register timeout");
        } catch {}
      }
    }
    open = open.filter((s) => s.readyState === 1); // OPEN
    if (open.length >= MAX_SOCKETS_PER_IP) {
      return new Response("too many connections from this address", { status: 429 });
    }
    if (!this.socketsPerIp.hit(ip, now)) {
      return new Response("too many new connections, retry later", {
        status: 429,
        headers: { "Retry-After": "60" },
      });
    }
    const [client, server] = Object.values(new WebSocketPair());
    // Hibernation API: the object can sleep while sockets stay open (free-plan friendly).
    this.ctx.acceptWebSocket(server, [tag]);
    server.serializeAttachment({ id: null, conn: crypto.randomUUID(), ip, since: now });
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, data) {
    const me = ws.deserializeAttachment() ?? {};
    if (!this.takeToken(me.conn)) {
      send(ws, error("rate_limited", "too many messages, connection closed"));
      return closeSocket(ws, CLOSE_POLICY, "rate limited");
    }
    const size = typeof data === "string" ? data.length : data.byteLength;
    if (size > MAX_MESSAGE) {
      send(ws, error("bad_request", "message too large"));
      return closeSocket(ws, CLOSE_TOO_BIG, "message too large");
    }
    if (typeof data !== "string") return send(ws, error("bad_request", "message not text"));
    let msg;
    try {
      msg = JSON.parse(data);
    } catch {
      return send(ws, error("bad_request", "invalid json"));
    }
    if (typeof msg !== "object" || msg === null) return send(ws, error("bad_request", "invalid message"));
    if (msg.type === "register") return this.register(ws, me, msg);
    if (!me.id) return send(ws, error("not_registered", "register first"));
    switch (msg.type) {
      case "connect":
        return this.connect(ws, me, msg.target);
      case "signal":
        return this.relay(ws, me.id, msg.session, msg.payload);
      case "leave":
        return this.leave(ws, me.id, msg.session);
      case "presence":
        return this.presence(ws, me, msg.ids);
      default:
        return send(ws, error("bad_request", "unknown message"));
    }
  }

  async webSocketClose(ws) {
    this.disconnect(ws);
  }

  async webSocketError(ws) {
    this.disconnect(ws);
  }

  /** Token bucket per socket: false once the socket sends faster than allowed. */
  takeToken(conn) {
    if (!conn) return true;
    const now = Date.now();
    let b = this.buckets.get(conn);
    if (!b) {
      // Entries of sockets closed while the object slept are never seen again: sweep them.
      if (this.buckets.size >= MAX_LIMITER_KEYS) this.sweepBuckets();
      b = { tokens: MESSAGE_BURST, at: now };
      this.buckets.set(conn, b);
    }
    b.tokens = Math.min(MESSAGE_BURST, b.tokens + ((now - b.at) / 1000) * MESSAGE_RATE);
    b.at = now;
    if (b.tokens < 1) return false;
    b.tokens -= 1;
    return true;
  }

  sweepBuckets() {
    const live = new Set(this.ctx.getWebSockets().map((s) => s.deserializeAttachment()?.conn));
    for (const conn of this.buckets.keys()) if (!live.has(conn)) this.buckets.delete(conn);
  }

  socketsOf(id) {
    return this.ctx.getWebSockets().filter((s) => s.deserializeAttachment()?.id === id);
  }

  async register(ws, me, msg) {
    const key = msg.device_key;
    if (typeof key !== "string" || key.length < 16 || key.length > 256) {
      return send(ws, error("bad_request", "invalid device key"));
    }
    if (me.id) return send(ws, error("bad_request", "already registered"));
    const hash = await sha256(key);
    let id = this.sql.exec("SELECT id FROM ids WHERE key_hash = ?", hash).toArray()[0]?.id;
    if (!id) {
      if (!this.newIdsPerIp.hit(me.ip ?? "unknown", Date.now())) {
        send(ws, error("rate_limited", "too many new devices from this address, retry later"));
        return closeSocket(ws, CLOSE_POLICY, "rate limited");
      }
      id = this.allocateId();
      this.sql.exec("INSERT INTO ids (key_hash, id) VALUES (?, ?)", hash, id);
    }
    // Same device again: the older connection is told why and hung up.
    for (const old of this.socketsOf(id)) {
      send(old, error("replaced", "this device connected again from another session"));
      try {
        old.close(1000, "replaced");
      } catch {}
      this.closeSessionsOf(id, "peer reconnected");
    }
    ws.serializeAttachment({ ...me, id });
    // STUN is enough until a session starts; sessions get TURN credentials.
    send(ws, { type: "registered", id, ice_servers: [STUN], features: ["presence"] });
  }

  /** Which of `ids` are online now (a device list's dots), rate limited per connection. */
  presence(ws, me, ids) {
    if (!Array.isArray(ids) || ids.length > MAX_PRESENCE_IDS || !ids.every((i) => /^\d{9}$/.test(i))) {
      return send(ws, error("bad_request", "invalid presence request"));
    }
    const now = Date.now();
    if (me.presenceAt && now - me.presenceAt < PRESENCE_INTERVAL_MS) {
      return send(ws, error("rate_limited", "presence asked too often"));
    }
    ws.serializeAttachment({ ...me, presenceAt: now });
    const online = new Set(
      this.ctx
        .getWebSockets()
        .map((s) => s.deserializeAttachment()?.id)
        .filter(Boolean),
    );
    send(ws, { type: "presence", online: ids.filter((i) => online.has(i)) });
  }

  allocateId() {
    for (;;) {
      const n = 100_000_000 + Math.floor(Math.random() * 900_000_000);
      const id = String(n);
      if (this.sql.exec("SELECT 1 FROM ids WHERE id = ?", id).toArray().length === 0) return id;
    }
  }

  nextSession() {
    const row = this.sql.exec("SELECT v FROM meta WHERE k = 'next_session'").toArray()[0];
    const session = row ? row.v : 1;
    this.sql.exec("INSERT OR REPLACE INTO meta (k, v) VALUES ('next_session', ?)", session + 1);
    return session;
  }

  async connect(ws, me, target) {
    const from = me.id;
    const ip = me.ip ?? "unknown";
    if (typeof target !== "string" || !/^[1-9][0-9]{8}$/.test(target)) {
      return send(ws, error("bad_request", "invalid target"));
    }
    if (target === from) return send(ws, error("self_connect", "cannot connect to yourself"));
    // Slows down id guessing and floods of attempts at one device (its password is checked
    // peer to peer, but every attempt starts here). Only allowed attempts are counted.
    const now = Date.now();
    const limits = [
      [this.connectsPerSocket, me.conn],
      [this.connectsPerIp, ip],
      [this.connectsPerIpTarget, `${ip}>${target}`],
      [this.connectsPerTarget, target],
    ];
    if (!limits.every(([l, k]) => l.allows(k, now))) {
      return send(ws, error("rate_limited", "too many connection attempts, retry later"));
    }
    for (const [l, k] of limits) l.hit(k, now);
    const [peer] = this.socketsOf(target);
    if (!peer) return send(ws, error("peer_offline", `peer ${target} is offline`));
    const session = this.nextSession();
    this.sql.exec("INSERT INTO sessions (session, a, b) VALUES (?, ?, ?)", session, from, target);
    // Bound the rows one device can leave behind (sessions never closed with `leave`):
    // the oldest are forgotten silently, an established WebRTC link does not need them.
    this.sql.exec(
      "DELETE FROM sessions WHERE a = ? AND session NOT IN (SELECT session FROM sessions WHERE a = ? ORDER BY session DESC LIMIT ?)",
      from,
      from,
      MAX_SESSIONS_PER_PEER,
    );
    // TURN credentials only for sessions between two online, registered peers, and only
    // so many per caller IP: beyond that the session still works where STUN suffices.
    const ice = this.turnPerIp.hit(ip, now) ? await this.iceServers() : [STUN];
    send(peer, { type: "incoming_session", session, from, ice_servers: ice });
    send(ws, { type: "session_created", session, peer: target, ice_servers: ice });
  }

  otherParty(session, me) {
    const row = this.sql.exec("SELECT a, b FROM sessions WHERE session = ?", session).toArray()[0];
    if (!row) return null;
    if (row.a === me) return row.b;
    if (row.b === me) return row.a;
    return null;
  }

  relay(ws, from, session, payload) {
    // Only the three WebRTC payloads of signaling-proto are relayed.
    if (typeof payload !== "object" || payload === null || !["offer", "answer", "candidate"].includes(payload.kind)) {
      return send(ws, error("bad_request", "invalid signal payload"));
    }
    const to = this.otherParty(session, from);
    if (!to) return send(ws, error("unknown_session", `unknown session ${session}`));
    for (const s of this.socketsOf(to)) send(s, { type: "signal", session, payload });
  }

  leave(ws, from, session) {
    const to = this.otherParty(session, from);
    if (!to) return send(ws, error("unknown_session", `unknown session ${session}`));
    this.sql.exec("DELETE FROM sessions WHERE session = ?", session);
    for (const s of this.socketsOf(to)) send(s, { type: "session_closed", session, reason: "peer left" });
  }

  /**
   * Tells the partners of `id` that its link to the server dropped or was replaced. The
   * sessions are kept: established ones go on peer to peer (apps from 1.5.3 ignore this
   * notice), and a later `leave` must still reach the partner (rows stay bounded per
   * device, see `connect`).
   */
  closeSessionsOf(id, reason) {
    const rows = this.sql.exec("SELECT session, a, b FROM sessions WHERE a = ? OR b = ?", id, id).toArray();
    for (const { session, a, b } of rows) {
      const other = a === id ? b : a;
      for (const s of this.socketsOf(other)) send(s, { type: "session_closed", session, reason });
    }
  }

  disconnect(ws) {
    const me = ws.deserializeAttachment();
    if (me?.conn) this.buckets.delete(me.conn);
    if (!me?.id) return;
    // A newer connection of the same device took over: its sessions are not ours to end.
    const newer = this.ctx.getWebSockets().some((s) => {
      const a = s.deserializeAttachment();
      return s !== ws && a?.id === me.id && a.conn !== me.conn;
    });
    if (!newer) this.closeSessionsOf(me.id, "peer disconnected");
  }

  /** STUN plus short-lived Cloudflare TURN credentials (UDP only: our WebRTC stack has no TCP/TLS TURN). */
  async iceServers() {
    const { TURN_URL: turnUrl, TURN_USERNAME: username, TURN_PASSWORD: password } = this.env;
    if (turnUrl && username && password) {
      const urls = turnUrl.split(/\s+/).filter((u) => u.startsWith("turn:") && !u.includes("transport=tcp"));
      return urls.length ? [STUN, { urls, username, credential: password }] : [STUN];
    }
    const { TURN_KEY_ID: keyId, TURN_KEY_API_TOKEN: token } = this.env;
    if (!keyId || !token) return [STUN];
    try {
      const response = await fetch(
        `https://rtc.live.cloudflare.com/v1/turn/keys/${keyId}/credentials/generate-ice-servers`,
        {
          method: "POST",
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
          body: JSON.stringify({ ttl: TURN_TTL }),
        },
      );
      if (!response.ok) throw new Error(`TURN API ${response.status}`);
      const { iceServers } = await response.json();
      return iceServers
        .map((s) => ({
          urls: s.urls.filter((u) => u.startsWith("stun:") || (u.startsWith("turn:") && u.includes("transport=udp"))),
          ...(s.username ? { username: s.username, credential: s.credential } : {}),
        }))
        .filter((s) => s.urls.length > 0);
    } catch (e) {
      console.warn("TURN credentials unavailable:", String(e));
      return [STUN];
    }
  }
}

/**
 * Fixed-window counter per key. `allows` checks, `hit` counts (and checks). Expired keys
 * are swept once per window and the table never holds more than MAX_LIMITER_KEYS keys
 * (beyond that the oldest key is dropped: a flood of addresses cannot exhaust memory).
 */
class Limiter {
  constructor(limit, windowMs) {
    this.limit = limit;
    this.windowMs = windowMs;
    this.entries = new Map();
    this.swept = 0;
  }

  allows(key, now) {
    const e = this.entries.get(key);
    return !e || now - e.start >= this.windowMs || e.count < this.limit;
  }

  hit(key, now) {
    if (now - this.swept >= this.windowMs) this.sweep(now);
    const e = this.entries.get(key);
    if (e && now - e.start < this.windowMs) {
      if (e.count >= this.limit) return false;
      e.count += 1;
      return true;
    }
    this.entries.delete(key);
    if (this.entries.size >= MAX_LIMITER_KEYS) {
      this.sweep(now);
      if (this.entries.size >= MAX_LIMITER_KEYS) this.entries.delete(this.entries.keys().next().value);
    }
    this.entries.set(key, { start: now, count: 1 });
    return true;
  }

  sweep(now) {
    this.swept = now;
    for (const [key, e] of this.entries) if (now - e.start >= this.windowMs) this.entries.delete(key);
  }
}

/** Rate-limit key of a client address: IPv4 as is, IPv6 by /64 (one subscriber's prefix). */
function ipKey(ip) {
  if (!ip) return "unknown";
  ip = ip.trim().toLowerCase();
  if (!ip.includes(":")) return ip;
  const mapped = ip.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/);
  if (mapped) return mapped[1];
  const [head, tail] = ip.split("::");
  const left = head ? head.split(":") : [];
  const right = tail ? tail.split(":") : [];
  const groups = tail === undefined ? left : [...left, ...Array(Math.max(0, 8 - left.length - right.length)).fill("0"), ...right];
  return `${groups.slice(0, 4).map((g) => g.replace(/^0+(?=.)/, "")).join(":")}::/64`;
}

function closeSocket(ws, code, reason) {
  try {
    ws.close(code, reason);
  } catch {}
}

function send(ws, obj) {
  try {
    ws.send(JSON.stringify(obj));
  } catch {}
}

function error(code, message) {
  return { type: "error", code, message };
}

async function sha256(text) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
