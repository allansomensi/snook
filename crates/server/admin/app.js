// Snook staff panel. Plain JavaScript, no dependencies; every value from the
// server is inserted with textContent (never as HTML).
"use strict";

const $ = (sel) => document.querySelector(sel);
let token = sessionStorage.getItem("snook-admin") || "";
let me = null;
let view = "dashboard";

function el(tag, attrs = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    e.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return e;
}

function toast(text) {
  const t = $("#toast");
  t.textContent = text;
  t.classList.remove("hidden");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => t.classList.add("hidden"), 3000);
}

async function api(path, options = {}) {
  const res = await fetch("/admin/api" + path, {
    method: options.method || "GET",
    headers: { "Content-Type": "application/json", ...(token ? { Authorization: "Bearer " + token } : {}) },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  let data = {};
  try { data = await res.json(); } catch (_) { /* empty */ }
  if (res.status === 401) { signOut(false); throw new Error(data.error || "Sign in again."); }
  if (!res.ok) throw new Error(data.error || "Request failed (" + res.status + ").");
  return data;
}

const fmtDate = (s) => (s ? new Date(s * 1000).toLocaleString() : "—");
const ago = (s) => {
  if (!s) return "—";
  const d = Math.max(0, Date.now() / 1000 - s);
  if (d < 90) return "just now";
  if (d < 5400) return Math.round(d / 60) + " min ago";
  if (d < 129600) return Math.round(d / 3600) + " h ago";
  return Math.round(d / 86400) + " days ago";
};
const until = (s) => (s >= 9e15 ? "permanently" : "until " + fmtDate(s));

function statusTags(u) {
  const now = Date.now() / 1000;
  return el("span", {},
    u.online ? el("span", { class: "tag on" }, "online") : null, " ",
    u.role !== "player" ? el("span", { class: "tag staff" }, u.role) : null, " ",
    u.banned_until && u.banned_until > now ? el("span", { class: "tag ban" }, "banned") : null, " ",
    u.muted_until && u.muted_until > now ? el("span", { class: "tag mute" }, "muted") : null);
}

function table(headers, rows) {
  return el("div", { class: "table-wrap" },
    el("table", {}, el("thead", {}, el("tr", {}, headers.map((h) => el("th", {}, h)))), el("tbody", {}, rows)));
}

function setView(name) {
  view = name;
  document.querySelectorAll("nav button").forEach((b) => b.classList.toggle("active", b.dataset.view === name));
  render();
}

async function render() {
  const main = $("#main");
  try {
    const content = await views[view]();
    main.replaceChildren(content);
  } catch (e) {
    main.replaceChildren(el("p", { class: "error" }, e.message));
  }
}

const views = {
  async dashboard() {
    const o = await api("/overview");
    $("#report-count").textContent = o.stored.open_reports || "";
    const stat = (n, l) => el("div", { class: "card stat" }, el("div", { class: "n" }, n), el("div", { class: "l" }, l));
    const h = Math.floor(o.uptime_secs / 3600), m = Math.floor((o.uptime_secs % 3600) / 60);
    const box = el("div", {},
      el("div", { class: "grid" },
        stat(o.online, "Online now"), stat(o.matches_live, "Games now"), stat(o.tables_open, "Open tables"),
        stat(o.tournaments, "Tournaments"), stat(o.stored.users, "Accounts"), stat(o.stored.new_users_7d, "New (7 days)"),
        stat(o.stored.active_users_7d, "Active (7 days)"), stat(o.stored.matches_24h, "Games (24 h)"),
        stat(o.stored.matches_total, "Games (all)"), stat(o.stored.open_reports, "Open reports"),
        stat(o.stored.banned, "Banned"), stat(h + "h " + m + "m", "Uptime · v" + o.version)),
      o.persistent ? null : el("p", { class: "error" }, "This server keeps everything in memory: nothing is saved."),
      o.online >= 4 && o.addresses === 1 ? el("p", { class: "error" }, "Every player comes from the same address: set SNOOK_CLIENT_IP_HEADER (see DEPLOY), or the per-address limits will block players.") : null);
    const announce = el("textarea", { rows: "2", maxlength: "200", placeholder: "Message to everyone in the lounge" });
    box.append(el("h2", {}, "Announcement"), announce,
      el("div", { class: "actions" }, el("button", {
        class: "primary", onclick: async () => {
          try { await api("/announce", { method: "POST", body: { text: announce.value } }); announce.value = ""; toast("Sent."); }
          catch (e) { toast(e.message); }
        },
      }, "Send to the lounge")));
    if (me && me.role === "admin") {
      const motd = el("input", { maxlength: "300" });
      motd.value = o.motd;
      box.append(el("h2", {}, "Welcome message"), motd, el("div", { class: "actions" }, el("button", {
        onclick: async () => {
          try { await api("/motd", { method: "POST", body: { text: motd.value } }); toast("Saved (until the server restarts)."); }
          catch (e) { toast(e.message); }
        },
      }, "Save")));
    }
    return box;
  },

  async users() {
    const box = el("div", {});
    const search = el("input", { placeholder: "Search by username", value: views.users.q || "" });
    const results = el("div", {});
    const load = async (page = 0) => {
      views.users.q = search.value;
      const d = await api("/users?q=" + encodeURIComponent(search.value) + "&page=" + page);
      const rows = d.users.map((u) => el("tr", { class: "click", onclick: () => showUser(u.username) },
        el("td", {}, u.username), el("td", {}, u.country || "—"), el("td", {}, u.rating),
        el("td", {}, u.games), el("td", {}, ago(u.last_seen)), el("td", {}, statusTags(u))));
      const pages = Math.ceil(d.total / d.per_page);
      results.replaceChildren(
        el("p", { class: "muted" }, d.total + " players"),
        table(["Player", "Country", "Rating", "Games", "Last seen", ""], rows),
        el("div", { class: "actions" },
          page > 0 ? el("button", { onclick: () => load(page - 1) }, "Previous") : null,
          page + 1 < pages ? el("button", { onclick: () => load(page + 1) }, "Next") : null));
    };
    search.addEventListener("keydown", (e) => { if (e.key === "Enter") load(); });
    box.append(el("div", { class: "row" }, search, el("button", { onclick: () => load() }, "Search")), results);
    await load();
    return box;
  },

  async reports() {
    const filter = views.reports.filter || "open";
    const d = await api("/reports?status=" + filter);
    const select = el("select", { onchange: () => { views.reports.filter = select.value; render(); } },
      ["open", "actioned", "dismissed", "all"].map((s) => {
        const o = el("option", { value: s }, s);
        if (s === filter) o.selected = true;
        return o;
      }));
    const list = d.reports.map((r) => el("div", { class: "card report" },
      el("div", { class: "head" },
        el("b", {}, "#" + r.id + " · " + r.reason),
        el("span", { class: "muted" }, fmtDate(r.created_at))),
      el("p", {}, el("a", { href: "#", onclick: (e) => { e.preventDefault(); showUser(r.target); } }, r.target),
        " reported by ", el("a", { href: "#", onclick: (e) => { e.preventDefault(); showUser(r.reporter); } }, r.reporter)),
      r.details ? el("p", {}, r.details) : null,
      r.context ? el("pre", {}, r.context) : null,
      r.status === 0
        ? el("div", { class: "actions" },
          el("button", { class: "primary", onclick: () => resolve(r.id, "actioned") }, "Action taken"),
          el("button", { onclick: () => resolve(r.id, "dismissed") }, "Dismiss"))
        : el("p", { class: "muted" }, (r.status === 1 ? "Actioned" : "Dismissed") + " by " + (r.resolved_by || "?") + (r.resolution ? ": " + r.resolution : ""))));
    return el("div", {}, el("div", { class: "row" }, el("span", {}, "Show"), select),
      list.length ? list : el("p", { class: "muted" }, "Nothing here."));
  },

  async live() {
    const d = await api("/live");
    const rooms = d.rooms.map((r) => el("tr", {},
      el("td", {}, "#" + r.id), el("td", {}, r.players[0] + " vs " + r.players[1]),
      el("td", {}, r.rated ? "rated" : "casual"), el("td", {}, r.tournament ? "yes" : ""),
      el("td", {}, r.shots), el("td", {}, r.finished ? "finished" : "playing")));
    const players = d.players.map((p) => el("tr", { class: "click", onclick: () => showUser(p.username) },
      el("td", {}, p.username), el("td", {}, p.rating), el("td", {}, p.status), el("td", {}, p.country || "—")));
    const tournaments = d.tournaments.map((t) => el("tr", {},
      el("td", {}, t.name), el("td", {}, t.host), el("td", {}, t.players.length + " / " + t.size),
      el("td", {}, t.state), el("td", {}, t.champion || "")));
    return el("div", {},
      el("div", { class: "actions" }, el("button", { onclick: render }, "Refresh")),
      el("h2", {}, "Games (" + d.rooms.length + ")"), table(["", "Players", "", "Tournament", "Shots", ""], rooms),
      el("h2", {}, "Tournaments (" + d.tournaments.length + ")"), table(["Name", "Host", "Players", "State", "Champion"], tournaments),
      el("h2", {}, "Online (" + d.players.length + ")"), table(["Player", "Rating", "Status", "Country"], players));
  },

  async audit() {
    const d = await api("/audit");
    return table(["When", "Staff", "Action", "Target", "Details"], d.entries.map((a) => el("tr", {},
      el("td", {}, fmtDate(a.at)), el("td", {}, a.staff), el("td", {}, a.action), el("td", {}, a.target), el("td", {}, a.details))));
  },
};

async function resolve(id, status) {
  const note = prompt("Note (optional)") || "";
  try { await api("/reports/" + id, { method: "POST", body: { status, note } }); toast("Saved."); render(); }
  catch (e) { toast(e.message); }
}

function modal(...children) {
  const m = $("#modal");
  $("#modal-body").replaceChildren(...children);
  m.classList.remove("hidden");
}
function closeModal() { $("#modal").classList.add("hidden"); }
$("#modal").addEventListener("click", (e) => { if (e.target.id === "modal") closeModal(); });

async function act(name, action, body) {
  try {
    await api("/users/" + encodeURIComponent(name) + "/" + action, { method: "POST", body: body || {} });
    toast("Done.");
    closeModal();
    showUser(name);
  } catch (e) { toast(e.message); }
}

function banDialog(name) {
  const hours = el("select", {}, [["24", "1 day"], ["72", "3 days"], ["168", "1 week"], ["720", "30 days"], ["0", "Permanent"]]
    .map(([v, l]) => el("option", { value: v }, l)));
  const reason = el("input", { placeholder: "Reason (shown to the player)", maxlength: "300" });
  modal(el("h2", {}, "Ban " + name), hours, reason, el("div", { class: "actions" },
    el("button", { class: "danger", onclick: () => act(name, "ban", { hours: Number(hours.value) || null, reason: reason.value }) }, "Ban"),
    el("button", { onclick: closeModal }, "Cancel")));
}

function muteDialog(name) {
  const minutes = el("select", {}, [["15", "15 minutes"], ["60", "1 hour"], ["1440", "1 day"], ["10080", "1 week"]]
    .map(([v, l]) => el("option", { value: v }, l)));
  const reason = el("input", { placeholder: "Reason", maxlength: "300" });
  modal(el("h2", {}, "Mute " + name), minutes, reason, el("div", { class: "actions" },
    el("button", { class: "danger", onclick: () => act(name, "mute", { minutes: Number(minutes.value), reason: reason.value }) }, "Mute"),
    el("button", { onclick: closeModal }, "Cancel")));
}

function roleDialog(name) {
  const role = el("select", {}, ["player", "moderator", "admin"].map((r) => el("option", { value: r }, r)));
  modal(el("h2", {}, "Role of " + name), role, el("div", { class: "actions" },
    el("button", { class: "primary", onclick: () => act(name, "role", { role: role.value }) }, "Save"),
    el("button", { onclick: closeModal }, "Cancel")));
}

function passwordDialog(name) {
  const pass = el("input", { type: "password", placeholder: "New password (8+ characters)", autocomplete: "new-password" });
  modal(el("h2", {}, "New password for " + name), pass, el("p", { class: "muted" }, "Signs the player out everywhere."),
    el("div", { class: "actions" },
      el("button", { class: "danger", onclick: () => act(name, "password", { password: pass.value }) }, "Set password"),
      el("button", { onclick: closeModal }, "Cancel")));
}

async function showUser(name) {
  view = "users";
  document.querySelectorAll("nav button").forEach((b) => b.classList.toggle("active", b.dataset.view === "users"));
  let d;
  try { d = await api("/users/" + encodeURIComponent(name)); } catch (e) { toast(e.message); return; }
  const u = d.user;
  const now = Date.now() / 1000;
  const banned = u.banned_until && u.banned_until > now;
  const muted = u.muted_until && u.muted_until > now;
  const isAdmin = me && me.role === "admin";
  const history = d.history.map((h) => el("tr", {},
    el("td", {}, fmtDate(h.at)), el("td", {}, h.opponent), el("td", {}, h.won ? "won" : "lost"),
    el("td", {}, h.reason.replaceAll("_", " ")), el("td", {}, h.rated ? (h.delta > 0 ? "+" : "") + h.delta : "casual"),
    el("td", {}, h.rating_after)));
  const reports = d.reports.map((r) => el("div", { class: "card report" },
    el("div", { class: "head" }, el("b", {}, r.reason + " · by " + r.reporter), el("span", { class: "muted" }, fmtDate(r.created_at))),
    r.details ? el("p", {}, r.details) : null, r.context ? el("pre", {}, r.context) : null));
  $("#main").replaceChildren(el("div", {},
    el("div", { class: "actions" }, el("button", { onclick: () => setView("users") }, "← Players")),
    el("div", { class: "card" },
      el("h2", {}, u.username, " ", statusTags(u)),
      el("p", {}, "Rating ", el("b", {}, u.rating), " (" + u.tier + ", peak " + u.peak_rating + ") · ",
        u.games + " rated games, " + u.wins + " wins · " + u.casual_games + " casual"),
      el("p", { class: "muted" }, "Country " + (u.country || "—") + " · joined " + fmtDate(u.created_at) + " · last seen " + ago(u.last_seen)),
      banned ? el("p", { class: "error" }, "Banned " + until(u.banned_until) + (u.ban_reason ? ": " + u.ban_reason : "")) : null,
      muted ? el("p", { class: "error" }, "Muted until " + fmtDate(u.muted_until)) : null,
      el("div", { class: "actions" },
        banned ? el("button", { onclick: () => act(u.username, "unban") }, "Unban") : el("button", { class: "danger", onclick: () => banDialog(u.username) }, "Ban…"),
        muted ? el("button", { onclick: () => act(u.username, "unmute") }, "Unmute") : el("button", { onclick: () => muteDialog(u.username) }, "Mute…"),
        u.online ? el("button", { onclick: () => act(u.username, "kick", { reason: prompt("Reason (optional)") || "" }) }, "Disconnect") : null,
        el("button", { onclick: () => act(u.username, "logout-all") }, "Sign out everywhere"),
        isAdmin ? el("button", { onclick: () => roleDialog(u.username) }, "Role…") : null,
        isAdmin ? el("button", { onclick: () => passwordDialog(u.username) }, "Reset password…") : null)),
    el("h2", {}, "Reports about this player (" + d.reports.length + ")"),
    reports.length ? reports : el("p", { class: "muted" }, "None."),
    el("h2", {}, "Recent games"),
    history.length ? table(["When", "Opponent", "Result", "How", "Rating", "After"], history) : el("p", { class: "muted" }, "None."),
    el("h2", {}, "Achievements (" + d.achievements.length + ")"),
    el("p", { class: "muted" }, d.achievements.map((a) => a[0]).join(", ") || "None.")));
}

function signOut(callServer = true) {
  if (callServer && token) api("/logout", { method: "POST" }).catch(() => {});
  token = "";
  me = null;
  sessionStorage.removeItem("snook-admin");
  $("#app").classList.add("hidden");
  $("#login").classList.remove("hidden");
}

async function start() {
  if (!token) { signOut(false); return; }
  try {
    me = await api("/me");
    $("#me").textContent = me.username + " · " + me.role;
    $("#login").classList.add("hidden");
    $("#app").classList.remove("hidden");
    render();
  } catch (_) { signOut(false); }
}

$("#login-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  $("#login-error").textContent = "";
  try {
    const d = await api("/login", { method: "POST", body: { username: $("#login-user").value, password: $("#login-pass").value } });
    token = d.token;
    sessionStorage.setItem("snook-admin", token);
    $("#login-pass").value = "";
    start();
  } catch (err) { $("#login-error").textContent = err.message; }
});
$("#logout").addEventListener("click", () => signOut(true));
document.querySelectorAll("nav button").forEach((b) => b.addEventListener("click", () => setView(b.dataset.view)));
start();
