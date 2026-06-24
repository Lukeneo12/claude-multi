import { useEffect, useState } from "react";
import { Config, TerminalInfo, getConfig, saveConfig, listTerminals } from "./api";

export default function App() {
  const [config, setConfig] = useState<Config | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [status, setStatus] = useState("");

  useEffect(() => {
    getConfig().then(setConfig);
    listTerminals().then(setTerminals);
  }, []);

  if (!config) return <p>Loading…</p>;

  const addProject = () => {
    const idx = config.projects.length + 1;
    setConfig({
      ...config,
      projects: [...config.projects, { id: `p${idx}`, label: `Project ${idx}`, path: "" }],
    });
  };

  const save = async () => {
    await saveConfig(config);
    setStatus("Saved — restart to refresh the tray menu.");
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
          <input value={a.label} onChange={(e) => {
            const accounts = [...config.accounts];
            accounts[i] = { ...a, label: e.target.value };
            setConfig({ ...config, accounts });
          }} />
          <code>{a.config_dir}</code>
        </div>
      ))}

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
