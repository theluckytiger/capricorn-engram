// The bridge to her core. In a plain browser (the UI alone, `npm run dev`) there's no core: calls are no-ops, so the
// overlay can still be worked on there.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type EventCallback } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { noteCall, noteEvent } from "./log";

export const inTauri = "__TAURI_INTERNALS__" in window;

// The webview's own right-click menu (Back, Refresh, Print, More tools) is a browser's, not hers: off in her windows,
// except where you type (cut, copy, paste).
if (inTauri)
  window.addEventListener("contextmenu", (e) => {
    if (!(e.target as HTMLElement | null)?.closest?.("input, textarea")) e.preventDefault();
  });

/** A call to her core that throws when it fails (and goes in the log when she's being tested). */
export async function call<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const at = performance.now();
  try {
    const result = await tauriInvoke<T>(cmd, args);
    noteCall(cmd, args, performance.now() - at, result);
    return result;
  } catch (e) {
    noteCall(cmd, args, performance.now() - at, undefined, e);
    throw e;
  }
}

export async function invoke<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!inTauri) return undefined;
  try {
    return await call<T>(cmd, args);
  } catch (e) {
    console.error(cmd, e);
    return undefined;
  }
}

export async function listen<T>(event: string, fn: EventCallback<T>) {
  if (!inTauri) return () => {};
  return tauriListen<T>(event, (e) => {
    noteEvent(event, e.payload, true);
    return fn(e);
  });
}

/** Only what's sent to this window. (`listen` hears events sent to any window: "come here", sent to the screen she
 *  was going to, also brought her back on the one she'd left, and there were two of her.) */
export async function listenHere<T>(event: string, fn: EventCallback<T>) {
  if (!inTauri) return () => {};
  return getCurrentWebviewWindow().listen<T>(event, (e) => {
    noteEvent(event, e.payload, false);
    return fn(e);
  });
}
