export interface BatteryState {
  detected: boolean;
  deviceName: string | null;
  connection: "connected" | "disconnected" | "unknown" | "notFound";
  bluetooth: "on" | "off" | "disabled" | "noAdapter" | "unknown";
  percent: number | null;
  source: string | null;
  precision: string | null;
  charging: "charging" | "discharging" | "notReported";
  transport: "bluetooth" | "usbDongle" | null;
  mouse: MouseState | null;
  lastUpdate: number | null;
  lastCheck: number;
}

export interface DeviceEntry {
  name: string;
  id: string;
  connected: boolean | null;
  matched: boolean;
}

export interface BatteryNode {
  name: string;
  instanceId: string;
  percent: number;
  matched: boolean;
}

export interface Diagnostics {
  generatedAt: number;
  radios: string[];
  classicDevices: DeviceEntry[];
  leDevices: DeviceEntry[];
  batteryNodes: BatteryNode[];
  gatt: string;
  hfp: string;
  hid: string;
  hidDevices: string[];
  mouse: string;
  log: string[];
}

export interface SettingsView {
  notifications: boolean;
  autostart: boolean;
  version: string;
}

export interface MouseState {
  name: string;
  percent: number | null;
  charging: boolean;
  asleep: boolean;
  lastUpdate: number | null;
}
