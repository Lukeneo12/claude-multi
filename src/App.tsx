import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Account, Config, Project, TerminalInfo, InheritSubdirStatus, InheritDecision,
  getConfig, saveConfig, listTerminals, getInheritStatus, setInheritDecision,
} from "./api";
import "./App.css";

// Account config dirs always live under `~/.claude-<suffix>` so they stay
// app-owned and never collide with the default `~/.claude`.
const CLAUDE_DIR_PREFIX = "~/.claude-";
const suffixOf = (dir: string) =>
  dir.startsWith(CLAUDE_DIR_PREFIX) ? dir.slice(CLAUDE_DIR_PREFIX.length) : dir;

const nextId = (ids: string[], prefix: string) => {
  const max = ids.reduce((m, id) => {
    const match = id.match(new RegExp(`^${prefix}(\\d+)$`));
    return match ? Math.max(m, parseInt(match[1], 10)) : m;
  }, 0);
  return `${prefix}${max + 1}`;
};

// `invoke` only works inside the Tauri webview; in a plain browser the IPC
// bridge is absent.
const IN_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Immutably replace item `i` of an array with a shallow-merged patch.
function updateAt<T>(arr: T[], i: number, patch: Partial<T>): T[] {
  return arr.map((item, j) => (j === i ? { ...item, ...patch } : item));
}

export default function App() {
  const [config, setConfig] = useState<Config | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [status, setStatus] = useState<{ kind: "ok" | "err"; text: string } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Inheritance panel: which account is inspected + its fetched per-subdir rows.
  const [inheritAccount, setInheritAccount] = useState<string>("");
  const [inheritRows, setInheritRows] = useState<InheritSubdirStatus[]>([]);
  const [inheritErr, setInheritErr] = useState<string | null>(null);

  useEffect(() => {
    if (!IN_TAURI) {
      setLoadError("Open this window from the claude-multi tray icon (run `npm run tauri dev`). It can't run in a plain browser.");
      return;
    }
    (async () => {
      try {
        const [cfg, terms] = await Promise.all([getConfig(), listTerminals()]);
        // Heal projects whose account is empty or points at a removed account
        // (e.g. migrated from the old default_account field): assign them to the
        // first account so they aren't silently invisible in the tray.
        const validIds = new Set(cfg.accounts.map((a) => a.id));
        const firstId = cfg.accounts[0]?.id ?? "";
        const projects = cfg.projects.map((p) =>
          validIds.has(p.account) ? p : { ...p, account: firstId }
        );
        setConfig({ ...cfg, projects });
        setInheritAccount(firstId);
        setTerminals(terms);
      } catch (e) {
        setLoadError(`Failed to load config: ${e}`);
      }
    })();
  }, []);

  // Auto-refresh the inheritance status whenever the inspected account changes.
  useEffect(() => {
    if (!IN_TAURI || !inheritAccount) {
      setInheritRows([]);
      return;
    }
    let cancelled = false;
    getInheritStatus(inheritAccount)
      .then((rows) => { if (!cancelled) { setInheritRows(rows); setInheritErr(null); } })
      .catch((e) => { if (!cancelled) { setInheritErr(`${e}`); setInheritRows([]); } });
    return () => { cancelled = true; };
  }, [inheritAccount]);

  // The window is hidden (not destroyed) on close, so this component persists.
  // Clear any stale status message each time the window regains focus.
  useEffect(() => {
    if (!IN_TAURI) return;
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => { if (focused) setStatus(null); })
      .then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);

  if (loadError) {
    return (
      <div className="app">
        <header className="app__header">
          <h1>claude-multi</h1>
          <p className="app__subtitle">Preferences</p>
        </header>
        <p className="banner banner--err">{loadError}</p>
      </div>
    );
  }

  if (!config) return <div className="app"><p className="muted">Loading…</p></div>;

  // Narrow once so the closures below operate on a non-null Config.
  const cfg: Config = config;
  const patch = (next: Partial<Config>) => setConfig({ ...cfg, ...next });
  const setAccounts = (accounts: Account[]) => patch({ accounts });
  const setProjects = (projects: Project[]) => patch({ projects });

  const addAccount = () => {
    const id = nextId(cfg.accounts.map((a) => a.id), "a");
    setAccounts([...cfg.accounts, { id, label: "New account", config_dir: `${CLAUDE_DIR_PREFIX}new` }]);
  };

  const addProject = () => {
    const id = nextId(cfg.projects.map((p) => p.id), "p");
    setProjects([...cfg.projects, { id, label: "", path: "", account: cfg.accounts[0]?.id ?? "" }]);
  };

  const browseProject = async (i: number) => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return; // cancelled
    const current = cfg.projects[i];
    const folderName = dir.split(/[\\/]/).pop() || current.label; // handle / and \
    const labelIsDefault = current.label.trim() === "" || /^Project \d+$/.test(current.label);
    setProjects(updateAt(cfg.projects, i, { path: dir, label: labelIsDefault ? folderName : current.label }));
  };

  // Persist a Merge/Skip decision for one subdir, then re-fetch to reflect it.
  const onInheritToggle = async (subdir: string, decision: InheritDecision) => {
    try {
      await setInheritDecision(inheritAccount, subdir, decision);
      setInheritRows(await getInheritStatus(inheritAccount));
      setInheritErr(null);
    } catch (e) {
      setInheritErr(`${e}`);
    }
  };

  const save = async () => {
    try {
      await saveConfig(cfg);
      setStatus({ kind: "ok", text: "Saved — the tray menu was updated." });
      window.setTimeout(() => setStatus(null), 3000);
    } catch (e) {
      setStatus({ kind: "err", text: `Save failed: ${e}` });
    }
  };

  return (
    <div className="app">
      <header className="app__header">
        <h1>claude-multi</h1>
        <p className="app__subtitle">Preferences</p>
      </header>

      <section className="card">
        <h2 className="card__title">Terminal</h2>
        <p className="card__hint">Which terminal the tray opens for sessions and logins.</p>
        <select
          className="select"
          value={cfg.terminal}
          onChange={(e) => patch({ terminal: e.target.value })}
        >
          {terminals.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
        </select>
      </section>

      <section className="card">
        <div className="card__head">
          <h2 className="card__title">Accounts</h2>
          <button className="btn btn--ghost" onClick={addAccount}>+ Add account</button>
        </div>
        <p className="card__hint">Each account is isolated in its own <code>{CLAUDE_DIR_PREFIX}…</code> config directory.</p>

        {cfg.accounts.length === 0 && <p className="muted">No accounts yet.</p>}
        {cfg.accounts.map((a, i) => (
          <div className="row" key={a.id}>
            <input
              className="input row__grow"
              placeholder="Label (e.g. Personal)"
              value={a.label}
              onChange={(e) => setAccounts(updateAt(cfg.accounts, i, { label: e.target.value }))}
            />
            <span className="affix">
              <span className="affix__prefix">{CLAUDE_DIR_PREFIX}</span>
              <input
                className="input affix__input"
                placeholder="suffix"
                value={suffixOf(a.config_dir)}
                onChange={(e) =>
                  setAccounts(updateAt(cfg.accounts, i, { config_dir: CLAUDE_DIR_PREFIX + e.target.value }))
                }
              />
            </span>
            <button
              className="btn btn--icon"
              title="Remove account"
              onClick={() => setAccounts(cfg.accounts.filter((_, j) => j !== i))}
            >✕</button>
          </div>
        ))}
      </section>

      <section className="card">
        <div className="card__head">
          <h2 className="card__title">Inheritance</h2>
          {cfg.accounts.length > 0 && (
            <select
              className="select"
              value={inheritAccount}
              onChange={(e) => setInheritAccount(e.target.value)}
            >
              {cfg.accounts.map((a) => <option key={a.id} value={a.id}>{a.label}</option>)}
            </select>
          )}
        </div>
        <p className="card__hint">Shared resources from <code>~/.claude</code> linked into each account. Toggle whether each subdir is merged in or skipped.</p>

        {cfg.accounts.length === 0 && <p className="muted">Add an account to see what it inherits.</p>}
        {inheritErr && <p className="banner banner--err">{inheritErr}</p>}
        {cfg.accounts.length > 0 && inheritRows.map((r) => {
          const mergeOn = r.decision === "merge" || (r.decision === null && r.status === "inherited");
          const skipOn = r.decision === "skip";
          return (
            <div className="row inherit-row" key={r.subdir}>
              <code className="inherit-row__name">{r.subdir}</code>
              <span className={`badge badge--${r.status}`}>{r.status}</span>
              {r.status !== "none" && (
                <span className="toggle">
                  <button
                    className={`toggle__btn ${mergeOn ? "toggle__btn--on" : ""}`}
                    onClick={() => onInheritToggle(r.subdir, "merge")}
                  >Merge</button>
                  <button
                    className={`toggle__btn ${skipOn ? "toggle__btn--on" : ""}`}
                    onClick={() => onInheritToggle(r.subdir, "skip")}
                  >Skip</button>
                </span>
              )}
            </div>
          );
        })}
        <p className="card__hint inherit-note"><code>plugins</code> — excluded by design.</p>
      </section>

      <section className="card">
        <div className="card__head">
          <h2 className="card__title">Projects</h2>
          <button className="btn btn--ghost" onClick={addProject} disabled={cfg.accounts.length === 0}>+ Add project</button>
        </div>
        <p className="card__hint">Each project belongs to one account and appears only under that account in the tray.</p>

        {cfg.projects.length === 0 && <p className="muted">No projects yet. Add one and pick a folder.</p>}
        {cfg.projects.map((p, i) => (
          <div className="row" key={p.id}>
            <input
              className="input row__label"
              placeholder="Label"
              value={p.label}
              onChange={(e) => setProjects(updateAt(cfg.projects, i, { label: e.target.value }))}
            />
            <input
              className="input row__grow"
              placeholder="/path/to/repo"
              value={p.path}
              onChange={(e) => setProjects(updateAt(cfg.projects, i, { path: e.target.value }))}
            />
            <button className="btn btn--secondary" onClick={() => browseProject(i)}>Browse…</button>
            <select
              className="select"
              value={p.account}
              onChange={(e) => setProjects(updateAt(cfg.projects, i, { account: e.target.value }))}
            >
              {!cfg.accounts.some((a) => a.id === p.account) && (
                <option value="">— account —</option>
              )}
              {cfg.accounts.map((a) => <option key={a.id} value={a.id}>{a.label}</option>)}
            </select>
            <button
              className="btn btn--icon"
              title="Remove project"
              onClick={() => setProjects(cfg.projects.filter((_, j) => j !== i))}
            >✕</button>
          </div>
        ))}
      </section>

      <footer className="app__footer">
        <button className="btn btn--primary" onClick={save}>Save</button>
        {status && <span className={`status status--${status.kind}`}>{status.text}</span>}
      </footer>
    </div>
  );
}
