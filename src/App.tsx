import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Config, TerminalInfo, getConfig, saveConfig, listTerminals } from "./api";

export default function App() {
  const [config, setConfig] = useState<Config | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [status, setStatus] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    // `invoke` only works inside the Tauri webview; in a plain browser the IPC
    // bridge is absent. Detect that early and show a clear message instead of
    // throwing uncaught rejections and hanging on "Loading…".
    const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (!inTauri) {
      setLoadError("Open this window from the claude-multi tray icon (run `npm run tauri dev`). It can't run in a plain browser.");
      return;
    }
    (async () => {
      try {
        const [cfg, terms] = await Promise.all([getConfig(), listTerminals()]);
        setConfig(cfg);
        setTerminals(terms);
      } catch (e) {
        setLoadError(`Failed to load config: ${e}`);
      }
    })();
  }, []);

  if (loadError) {
    return (
      <main style={{ padding: 16, fontFamily: "system-ui" }}>
        <h2>claude-multi · Preferences</h2>
        <p style={{ color: "#b00" }}>{loadError}</p>
      </main>
    );
  }

  if (!config) return <p style={{ padding: 16, fontFamily: "system-ui" }}>Loading…</p>;

  // Account config dirs are always under `~/.claude-<suffix>` so they stay
  // app-owned and never collide with the default `~/.claude`. The UI shows the
  // fixed prefix and lets the user edit only the free suffix.
  const CLAUDE_DIR_PREFIX = "~/.claude-";
  const suffixOf = (dir: string) =>
    dir.startsWith(CLAUDE_DIR_PREFIX) ? dir.slice(CLAUDE_DIR_PREFIX.length) : dir;

  const addAccount = () => {
    const maxSuffix = config.accounts.reduce((max, a) => {
      const m = a.id.match(/^a(\d+)$/);
      return m ? Math.max(max, parseInt(m[1], 10)) : max;
    }, 0);
    const idx = maxSuffix + 1;
    setConfig({
      ...config,
      accounts: [
        ...config.accounts,
        { id: `a${idx}`, label: "New account", config_dir: `${CLAUDE_DIR_PREFIX}new${idx}` },
      ],
    });
  };

  const addProject = () => {
    const maxSuffix = config.projects.reduce((max, p) => {
      const m = p.id.match(/^p(\d+)$/);
      return m ? Math.max(max, parseInt(m[1], 10)) : max;
    }, 0);
    const idx = maxSuffix + 1;
    setConfig({
      ...config,
      projects: [...config.projects, { id: `p${idx}`, label: `Project ${idx}`, path: "" }],
    });
  };

  const browseProject = async (i: number) => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return; // cancelled
    const projects = [...config.projects];
    const current = projects[i];
    const folderName = dir.split("/").pop() || current.label;
    const labelIsDefault = current.label.trim() === "" || /^Project \d+$/.test(current.label);
    projects[i] = { ...current, path: dir, label: labelIsDefault ? folderName : current.label };
    setConfig({ ...config, projects });
  };

  const save = async () => {
    try {
      await saveConfig(config);
      setStatus("Saved — restart to refresh the tray menu.");
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    }
  };

  return (
    <main style={{ padding: 16, fontFamily: "system-ui" }}>
      <h2>claude-multi · Preferences</h2>

      <label>Terminal:{" "}
        <select value={config.terminal} onChange={(e) => setConfig({ ...config, terminal: e.target.value })}>
          {terminals.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
        </select>
      </label>

      <h3>Accounts</h3>
      {config.accounts.map((a, i) => (
        <div key={a.id}>
          <input placeholder="Label" value={a.label} onChange={(e) => {
            const accounts = [...config.accounts];
            accounts[i] = { ...a, label: e.target.value };
            setConfig({ ...config, accounts });
          }} />
          <code>{CLAUDE_DIR_PREFIX}</code>
          <input placeholder="suffix" value={suffixOf(a.config_dir)} onChange={(e) => {
            const accounts = [...config.accounts];
            accounts[i] = { ...a, config_dir: CLAUDE_DIR_PREFIX + e.target.value };
            setConfig({ ...config, accounts });
          }} />
          <button onClick={() => setConfig({ ...config, accounts: config.accounts.filter((_, j) => j !== i) })}>✕</button>
        </div>
      ))}
      <button onClick={addAccount}>Add account</button>

      <h3>Projects</h3>
      {config.projects.map((p, i) => (
        <div key={p.id}>
          <input placeholder="Label" value={p.label} onChange={(e) => {
            const projects = [...config.projects];
            projects[i] = { ...p, label: e.target.value };
            setConfig({ ...config, projects });
          }} />
          <input placeholder="/path/to/repo" value={p.path} onChange={(e) => {
            const projects = [...config.projects];
            projects[i] = { ...p, path: e.target.value };
            setConfig({ ...config, projects });
          }} />
          <button onClick={() => browseProject(i)}>Browse…</button>
          <button onClick={() => setConfig({ ...config, projects: config.projects.filter((_, j) => j !== i) })}>✕</button>
        </div>
      ))}
      <button onClick={addProject}>Add project</button>

      <div style={{ marginTop: 16 }}>
        <button onClick={save}>Save</button> <span>{status}</span>
      </div>
    </main>
  );
}
