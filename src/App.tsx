import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BatteryState, Diagnostics as Diag, SettingsView } from "./types";

function levelClass(p: number): string {
  if (p >= 50) return "green";
  if (p >= 20) return "yellow";
  if (p >= 10) return "red";
  return "red pulse";
}

function ago(ts: number | null, now: number): string {
  if (!ts) return "Never";
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 5) return "Just now";
  if (s < 60) return `${s} s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  return new Date(ts).toLocaleTimeString();
}

const BT_LABEL: Record<BatteryState["bluetooth"], string> = {
  on: "On",
  off: "Off",
  disabled: "Disabled",
  noAdapter: "No adapter",
  unknown: "Unknown",
};
const CONN_LABEL: Record<BatteryState["connection"], string> = {
  connected: "Connected",
  disconnected: "Disconnected",
  unknown: "Unknown",
  notFound: "Not paired",
};
const CHARGE_LABEL: Record<BatteryState["charging"], string> = {
  charging: "Charging",
  discharging: "On battery",
  notReported: "Not reported by device",
};

function Gauge({ st }: { st: BatteryState | null }) {
  const p = st?.percent ?? null;
  const r = 92;
  const c = 2 * Math.PI * r;
  const cls = p === null ? "none" : levelClass(p);
  return (
    <div className={`gauge ${cls}`}>
      <svg viewBox="0 0 220 220" aria-hidden>
        <circle cx="110" cy="110" r={r} className="track" />
        {p !== null && (
          <circle
            cx="110"
            cy="110"
            r={r}
            className="arc"
            strokeDasharray={c}
            strokeDashoffset={c * (1 - p / 100)}
            transform="rotate(-90 110 110)"
          />
        )}
      </svg>
      <div className="gauge-center">
        {p !== null ? (
          <>
            <div className="pct">
              {p}
              <span>%</span>
            </div>
            <div className="sub">{st?.precision === "10% steps" ? "Reported in 10% steps" : "Battery"}</div>
          </>
        ) : (
          <div className="na">{unavailableText(st)}</div>
        )}
      </div>
    </div>
  );
}

function unavailableText(st: BatteryState | null): string {
  if (!st || st.lastCheck === 0) return "Searching…";
  if (st.transport !== "usbDongle") {
    if (st.bluetooth === "off" || st.bluetooth === "disabled") return "Bluetooth is off";
    if (st.bluetooth === "noAdapter" && st.connection !== "notFound") return "No Bluetooth adapter";
  }
  if (st.connection === "disconnected")
    return st.transport === "usbDongle" ? "Headset is off" : "Headset disconnected";
  if (st.bluetooth === "noAdapter" && st.connection === "notFound") return "No Arctis 3 / Nova 3P / 3X found";
  if (st.connection === "notFound") return "No Arctis 3 / Nova 3P / 3X found";
  return "Battery percentage unavailable";
}

function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className="toggle">
      <span>{label}</span>
      <input type="checkbox" checked={on} onChange={(e) => onChange(e.target.checked)} />
      <i />
    </label>
  );
}

function DiagnosticsPanel() {
  const [d, setD] = useState<Diag | null>(null);
  const [copied, setCopied] = useState(false);
  const load = () => invoke<Diag>("get_diagnostics").then(setD);
  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);
  const rescan = async () => {
    await invoke("refresh");
    setTimeout(load, 1500);
  };
  const copy = async () => {
    await navigator.clipboard.writeText(JSON.stringify(d, null, 2));
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  if (!d) return <div className="diag">Loading…</div>;
  const conn = (c: boolean | null) => (c === null ? "?" : c ? "connected" : "not connected");
  return (
    <div className="diag">
      <div className="diag-actions">
        <button onClick={rescan}>Rescan</button>
        <button onClick={copy}>{copied ? "Copied" : "Copy report"}</button>
      </div>
      <h4>Bluetooth radio</h4>
      {d.radios.length ? d.radios.map((r) => <p key={r}>{r}</p>) : <p className="muted">No Bluetooth radio found</p>}
      <h4>GATT Battery Service</h4>
      <p>{d.gatt || "—"}</p>
      <h4>Hands-Free battery report</h4>
      <p>{d.hfp || "—"}</p>
      <h4>SteelSeries 2.4 GHz dongle</h4>
      <p>{d.hid || "—"}</p>
      {d.hidDevices.map((x) => (
        <p key={x} className="muted">
          {x}
        </p>
      ))}
      <h4>Paired Classic devices ({d.classicDevices.length})</h4>
      {d.classicDevices.map((x) => (
        <p key={x.id} className={x.matched ? "hit" : ""}>
          {x.name} <span className="muted">· {conn(x.connected)}</span>
        </p>
      ))}
      <h4>Paired LE devices ({d.leDevices.length})</h4>
      {d.leDevices.length === 0 && <p className="muted">None</p>}
      {d.leDevices.map((x) => (
        <p key={x.id} className={x.matched ? "hit" : ""}>
          {x.name} <span className="muted">· {conn(x.connected)}</span>
        </p>
      ))}
      <h4>Devices reporting battery to Windows</h4>
      {d.batteryNodes.length === 0 && <p className="muted">None</p>}
      {d.batteryNodes.map((x) => (
        <p key={x.instanceId} className={x.matched ? "hit" : ""}>
          {x.name || x.instanceId} <span className="muted">· {x.percent}%</span>
        </p>
      ))}
      {d.log.length > 0 && (
        <>
          <h4>Log</h4>
          {d.log.slice(-10).map((l, i) => (
            <p key={i} className="muted">
              {l}
            </p>
          ))}
        </>
      )}
      <p className="muted small">Last scan: {new Date(d.generatedAt).toLocaleTimeString()}</p>
    </div>
  );
}

export default function App() {
  const [st, setSt] = useState<BatteryState | null>(null);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [now, setNow] = useState(Date.now());
  const [showDiag, setShowDiag] = useState(false);

  useEffect(() => {
    invoke<BatteryState>("get_state").then(setSt);
    invoke<SettingsView>("get_settings").then(setSettings);
    const un = listen<BatteryState>("battery-update", (e) => setSt(e.payload));
    const t = setInterval(() => {
      if (document.visibilityState === "visible") setNow(Date.now());
    }, 1000);
    return () => {
      un.then((f) => f());
      clearInterval(t);
    };
  }, []);

  const setAutostart = async (v: boolean) => {
    const actual = await invoke<boolean>("set_autostart", { enabled: v }).catch(() => !v);
    setSettings((s) => (s ? { ...s, autostart: actual } : s));
  };
  const setNotify = async (v: boolean) => {
    await invoke("set_notifications", { enabled: v });
    setSettings((s) => (s ? { ...s, notifications: v } : s));
  };

  const btOk = st?.bluetooth === "on";
  const connOk = st?.connection === "connected";

  return (
    <div className="app">
      <header className="titlebar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <span className="logo" />
          ARCTIS <em>BATTERY</em>
        </div>
        <div className="win-btns">
          <button title="Minimize to tray" onClick={() => invoke("hide_window")}>
            <svg viewBox="0 0 10 10"><path d="M1 5h8" /></svg>
          </button>
          <button title="Close to tray" className="close" onClick={() => invoke("hide_window")}>
            <svg viewBox="0 0 10 10"><path d="M1 1l8 8M9 1l-8 8" /></svg>
          </button>
        </div>
      </header>

      <main>
        <div className="device">
          <div className="device-name">{st?.deviceName ?? "SteelSeries Arctis"}</div>
          <div className={`pill ${connOk ? "ok" : ""}`}>{st ? CONN_LABEL[st.connection] : "…"}</div>
        </div>

        <Gauge st={st} />

        <div className="rows">
          <div className="row">
            <span>Link</span>
            <b>{st?.transport === "usbDongle" ? "2.4 GHz USB dongle" : st?.transport === "bluetooth" ? "Bluetooth" : "—"}</b>
          </div>
          <div className="row">
            <span>Bluetooth</span>
            <b className={btOk ? "ok" : st?.transport === "usbDongle" ? "" : "warn"}>{st ? BT_LABEL[st.bluetooth] : "…"}</b>
          </div>
          <div className="row">
            <span>Charging</span>
            <b className={st?.charging === "charging" ? "ok" : ""}>{st ? CHARGE_LABEL[st.charging] : "…"}</b>
          </div>
          <div className="row">
            <span>Source</span>
            <b className="src">{st?.source ?? "—"}</b>
          </div>
          <div className="row">
            <span>Last update</span>
            <b>{ago(st?.lastUpdate ?? null, now)}</b>
          </div>
        </div>

        {settings && (
          <div className="card">
            <Toggle
              label={navigator.userAgent.includes("Mac") ? "Open at login" : "Start with Windows"}
              on={settings.autostart}
              onChange={setAutostart}
            />
            <Toggle label="Low battery alerts (20% / 10%)" on={settings.notifications} onChange={setNotify} />
          </div>
        )}

        <button className="diag-toggle" onClick={() => setShowDiag((v) => !v)}>
          Bluetooth diagnostics {showDiag ? "▴" : "▾"}
        </button>
        {showDiag && <DiagnosticsPanel />}
      </main>

      <footer>
        <span>v{settings?.version ?? ""}</span>
        <button className="link" onClick={() => invoke("quit_app")}>
          Quit
        </button>
      </footer>
    </div>
  );
}
