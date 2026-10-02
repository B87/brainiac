/**
 * The whole app over the fake backend, for `app.spec.ts`. Tauri's own mocks
 * stand in for IPC, events, and the window.
 */
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { FakeBackend } from "./fake";

mockWindows("main");
const fake = new FakeBackend();
mockIPC((cmd, args) => fake.handle(cmd, args), { shouldMockEvents: true });

declare global {
  interface Window {
    fake: FakeBackend;
    emitEvent: (name: string, payload: unknown) => Promise<void>;
  }
}
window.fake = fake;
window.emitEvent = (name, payload) => emit(name, payload);
fake.emit = (name, payload) => void emit(name, payload);

// The app's modules talk to Tauri as soon as they load, so load them after the mocks.
const { default: App } = await import("../../src/App");
await import("./app.css");
const root = document.getElementById("root");
if (root)
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
